use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError, VmExitError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::msr::{GuestMsrAccessPolicy, GuestMsrValueSet, MsrIndex};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE, LONG_MODE_PD_ADDR};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::PortIoBus;
use crate::privilege::{
    LongModePrivilegeLayout, PRIVILEGE_IDT_ADDR, PRIVILEGE_KERNEL_ENTRY, PRIVILEGE_PT_ADDR,
    PRIVILEGE_TERMINAL_HANDLER, PRIVILEGE_TSS_RSP0, PRIVILEGE_USER_CODE_SELECTOR,
    PRIVILEGE_USER_DATA_SELECTOR, PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_STACK,
};
use crate::syscall::{
    EFER_SYSCALL_ENABLE, MSR_EFER, MSR_LSTAR, MSR_SFMASK, MSR_STAR, SYSCALL_KERNEL_ENTRY,
    SYSCALL_KERNEL_STACK, SYSCALL_LSTAR_VALUE, SYSCALL_SFMASK_VALUE, SYSCALL_STAR_VALUE,
};
use crate::vcpu::{PortIoExit, Vcpu, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

pub const COPYIN_GOOD_POINTER: u64 = 0xa100;
pub const COPYIN_RESULT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xa180);
pub const COPYIN_FAULT_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb000);
pub const COPYIN_BAD_POINTER: u64 = 0x40_0000;
pub const COPYIN_PAGE_FAULT_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_4000);
pub const COPYIN_PAGE_FAULT_VECTOR: u8 = 14;
pub const COPYIN_FAULT_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 13;
pub const COPYIN_FIXUP_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 26;
pub const COPYIN_FIRST_RETURN_RIP: u64 = PRIVILEGE_USER_ENTRY.get() + 12;
pub const COPYIN_SECOND_RETURN_RIP: u64 = PRIVILEGE_USER_ENTRY.get() + 37;
pub const COPYIN_TERMINAL_RETURN_RIP: u64 = PRIVILEGE_USER_ENTRY.get() + 43;
pub const COPYIN_TERMINAL_HLT_RIP: u64 = PRIVILEGE_TERMINAL_HANDLER.get() + 5;
pub const COPYIN_GOOD_VALUE: u8 = 0x5a;
pub const COPYIN_EFAULT: u64 = (-14_i64) as u64;
pub const COPYIN_PROOF: &[u8; 3] = b"GFD";

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;
const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_RFLAGS_RESERVED: u64 = 1 << 1;
const X86_RFLAGS_IF: u64 = 1 << 9;
const X86_RFLAGS_RF: u64 = 1 << 16;
const COPYIN_EXIT_BUDGET: u32 = 4;
const PRIVILEGE_FRAME_BYTES: u64 = 5 * 8;
const PAGE_FAULT_OBSERVATION_BYTES: usize = 40;
const PAGE_FAULT_GATE_SIZE: u64 = 16;
const PAGE_FAULT_ERROR_CODE: u64 = 0;
const PAGE_FAULT_SAVED_RFLAGS: u64 = X86_RFLAGS_RESERVED | X86_RFLAGS_RF;
const BAD_POINTER_PD_INDEX: u64 = (COPYIN_BAD_POINTER >> 21) & 0x1ff;

const KERNEL_BOOT_BYTES: [u8; 41] = [
    0xfa, // cli
    0x66, 0xb8, 0x28, 0x00, // mov ax, 0x28
    0x0f, 0x00, 0xd8, // ltr ax
    0x6a, 0x1b, // push user SS
    0x48, 0xb8, 0x00, 0xd0, 0x1f, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs user RSP, rax
    0x50, // push rax
    0x68, 0x02, 0x02, 0x00, 0x00, // push 0x202 user RFLAGS
    0x6a, 0x23, // push user CS
    0x48, 0xb8, 0x00, 0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs user RIP, rax
    0x50, // push rax
    0x48, 0xcf, // iretq
];

const USER_BYTES: [u8; 43] = [
    0x48, 0xbf, 0x00, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs rdi, 0xa100
    0x0f, 0x05, // syscall: valid one-byte copyin
    0x48, 0xbb, 0x80, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs rbx, 0xa180
    0x48, 0x89, 0x03, // mov [rbx], rax
    0x48, 0xbf, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs rdi, 0x400000
    0x0f, 0x05, // syscall: canonical but unmapped pointer
    0x48, 0x89, 0x43, 0x08, // mov [rbx+8], rax
    0xcd, 0x81, // int 0x81 terminal gate
];

const COPYIN_HANDLER_BYTES: [u8; 32] = [
    0x49, 0x89, 0xe2, // mov r10, rsp: preserve user stack
    0x48, 0xbc, 0x00, 0xe0, 0x1f, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs rsp, 0x1fe000
    0x0f, 0xb6, 0x07, // movzx eax, byte ptr [rdi] -- unique fault site
    0x49, 0x89, 0xc0, // mov r8, rax
    0xb0, b'G', // mov al, 'G'
    0xe6, 0xe9, // out 0xe9, al
    0x4c, 0x89, 0xc0, // mov rax, r8
    0x4c, 0x89, 0xd4, // fixup: mov rsp, r10
    0x48, 0x0f, 0x07, // sysretq
];

const PAGE_FAULT_HANDLER_BYTES: [u8; 86] = [
    0x0f, 0x20, 0xd0, // mov rax, cr2
    0x48, 0xba, 0x00, 0xb0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs rdx, 0xb000
    0x48, 0x89, 0x02, // mov [rdx], rax: CR2
    0x48, 0x8b, 0x04, 0x24, // mov rax, [rsp]: error code
    0x48, 0x89, 0x42, 0x08, // mov [rdx+8], rax
    0x48, 0x8b, 0x44, 0x24, 0x08, // mov rax, [rsp+8]: fault RIP
    0x48, 0x89, 0x42, 0x10, // mov [rdx+16], rax
    0x48, 0x8b, 0x44, 0x24, 0x10, // mov rax, [rsp+16]: fault CS
    0x48, 0x89, 0x42, 0x18, // mov [rdx+24], rax
    0x48, 0x8b, 0x44, 0x24, 0x18, // mov rax, [rsp+24]: saved RFLAGS
    0x48, 0x89, 0x42, 0x20, // mov [rdx+32], rax
    0x48, 0xb8, 0x1a, 0x20, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs rax, fixup
    0x48, 0x89, 0x44, 0x24, 0x08, // mov [rsp+8], rax
    0x49, 0xc7, 0xc0, 0xf2, 0xff, 0xff, 0xff, // mov r8, -14
    0xb0, b'F', // mov al, 'F'
    0xe6, 0xe9, // out 0xe9, al
    0x4c, 0x89, 0xc0, // mov rax, r8
    0x48, 0x83, 0xc4, 0x08, // add rsp, 8: discard #PF error code
    0x48, 0xcf, // iretq to fixed copyin fixup
];

const TERMINAL_HANDLER_BYTES: [u8; 5] = [
    0xb0, b'D', // mov al, 'D'
    0xe6, 0xe9, // out 0xe9, al
    0xf4, // hlt in ring0
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyinPageFaultObservation {
    cr2: u64,
    error_code: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
}

impl CopyinPageFaultObservation {
    #[must_use]
    pub const fn cr2(self) -> u64 {
        self.cr2
    }

    #[must_use]
    pub const fn error_code(self) -> u64 {
        self.error_code
    }

    #[must_use]
    pub const fn rip(self) -> u64 {
        self.rip
    }

    #[must_use]
    pub const fn cs(self) -> u64 {
        self.cs
    }

    #[must_use]
    pub const fn rflags(self) -> u64 {
        self.rflags
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyinTerminalFrame {
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

impl CopyinTerminalFrame {
    #[must_use]
    pub const fn rip(self) -> u64 {
        self.rip
    }

    #[must_use]
    pub const fn cs(self) -> u64 {
        self.cs
    }

    #[must_use]
    pub const fn rflags(self) -> u64 {
        self.rflags
    }

    #[must_use]
    pub const fn rsp(self) -> u64 {
        self.rsp
    }

    #[must_use]
    pub const fn ss(self) -> u64 {
        self.ss
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultSafeCopyinGuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
    good_result: u64,
    bad_result: u64,
    page_fault: CopyinPageFaultObservation,
    terminal_frame: CopyinTerminalFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    final_cr2: u64,
    efer: u64,
    star: u64,
    lstar: u64,
    sfmask: u64,
    good_page_pte: u64,
    fault_handler_pte: u64,
    fault_observation_pte: u64,
    bad_pd_entry: u64,
}

impl FaultSafeCopyinGuestResult {
    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn report(&self) -> VmExitReport {
        self.report
    }

    #[must_use]
    pub const fn good_result(&self) -> u64 {
        self.good_result
    }

    #[must_use]
    pub const fn bad_result(&self) -> u64 {
        self.bad_result
    }

    #[must_use]
    pub const fn page_fault(&self) -> CopyinPageFaultObservation {
        self.page_fault
    }

    #[must_use]
    pub const fn terminal_frame(&self) -> CopyinTerminalFrame {
        self.terminal_frame
    }

    #[must_use]
    pub const fn terminal_rsp(&self) -> u64 {
        self.terminal_rsp
    }

    #[must_use]
    pub const fn terminal_cs(&self) -> u16 {
        self.terminal_cs
    }

    #[must_use]
    pub const fn terminal_rflags(&self) -> u64 {
        self.terminal_rflags
    }

    #[must_use]
    pub const fn final_cr2(&self) -> u64 {
        self.final_cr2
    }

    #[must_use]
    pub const fn efer(&self) -> u64 {
        self.efer
    }

    #[must_use]
    pub const fn star(&self) -> u64 {
        self.star
    }

    #[must_use]
    pub const fn lstar(&self) -> u64 {
        self.lstar
    }

    #[must_use]
    pub const fn sfmask(&self) -> u64 {
        self.sfmask
    }

    #[must_use]
    pub const fn good_page_pte(&self) -> u64 {
        self.good_page_pte
    }

    #[must_use]
    pub const fn fault_handler_pte(&self) -> u64 {
        self.fault_handler_pte
    }

    #[must_use]
    pub const fn fault_observation_pte(&self) -> u64 {
        self.fault_observation_pte
    }

    #[must_use]
    pub const fn bad_pd_entry(&self) -> u64 {
        self.bad_pd_entry
    }
}

#[derive(Debug, Clone, Copy)]
struct RuntimeState {
    good_result: u64,
    bad_result: u64,
    page_fault: CopyinPageFaultObservation,
    terminal_frame: CopyinTerminalFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    final_cr2: u64,
    msrs: [u64; 4],
    good_page_pte: u64,
    fault_handler_pte: u64,
    fault_observation_pte: u64,
    bad_pd_entry: u64,
}

pub fn run_fault_safe_copyin_guest(config: VmConfig) -> Result<FaultSafeCopyinGuestResult, Error> {
    let kernel = FlatGuestImage::new(
        PRIVILEGE_KERNEL_ENTRY,
        PRIVILEGE_KERNEL_ENTRY,
        &KERNEL_BOOT_BYTES,
    )?;
    let user = FlatGuestImage::new(PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_ENTRY, &USER_BYTES)?;
    let copyin_handler = FlatGuestImage::new(
        SYSCALL_KERNEL_ENTRY,
        SYSCALL_KERNEL_ENTRY,
        &COPYIN_HANDLER_BYTES,
    )?;
    let page_fault_handler = FlatGuestImage::new(
        COPYIN_PAGE_FAULT_HANDLER,
        COPYIN_PAGE_FAULT_HANDLER,
        &PAGE_FAULT_HANDLER_BYTES,
    )?;
    let terminal_handler = FlatGuestImage::new(
        PRIVILEGE_TERMINAL_HANDLER,
        PRIVILEGE_TERMINAL_HANDLER,
        &TERMINAL_HANDLER_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModePrivilegeLayout::new(memory.region())
        .expect("fixed bounded fault-safe copyin privilege layout remains valid");
    layout.install_tables(&mut memory)?;
    install_page_fault_gate(&mut memory)?;
    kernel.load(&mut memory)?;
    user.load(&mut memory)?;
    copyin_handler.load(&mut memory)?;
    page_fault_handler.load(&mut memory)?;
    terminal_handler.load(&mut memory)?;
    memory.write(GuestPhysAddr::new(COPYIN_GOOD_POINTER), &[COPYIN_GOOD_VALUE])?;
    memory.write(COPYIN_RESULT_ADDR, &[0; 16])?;
    memory.write(
        COPYIN_FAULT_OBSERVATION_ADDR,
        &[0; PAGE_FAULT_OBSERVATION_BYTES],
    )?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(&layout)?;
    let msrs = configure_copyin_msrs(&backend, &vcpu)?;

    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, COPYIN_EXIT_BUDGET)?;
    if execution.io_exits().len() != COPYIN_PROOF.len() {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "fault-safe copyin proof output count",
            expected_reason: crate::vcpu::VcpuExit::Io.reason(),
            actual_reason: execution.report().exit().reason(),
        }));
    }
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != COPYIN_PROOF {
        return Err(verification_error(
            "fault-safe copyin proof",
            format!("expected {COPYIN_PROOF:?}, got {proof:?}"),
        ));
    }

    let registers = vcpu.capture_register_snapshot()?;
    let terminal_regs = vcpu.registers()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered copyin guest memory remains VM-owned");
    let (good_result, bad_result) = read_results(guest_memory)?;
    let state = RuntimeState {
        good_result,
        bad_result,
        page_fault: read_page_fault_observation(guest_memory)?,
        terminal_frame: read_terminal_frame(guest_memory)?,
        terminal_rsp: registers.rsp(),
        terminal_cs: special.cs().selector(),
        terminal_rflags: terminal_regs.rflags,
        final_cr2: special.cr2(),
        msrs,
        good_page_pte: read_pte(guest_memory, COPYIN_GOOD_POINTER)?,
        fault_handler_pte: read_pte(guest_memory, COPYIN_PAGE_FAULT_HANDLER.get())?,
        fault_observation_pte: read_pte(guest_memory, COPYIN_FAULT_OBSERVATION_ADDR.get())?,
        bad_pd_entry: read_bad_pointer_pd_entry(guest_memory)?,
    };
    validate_runtime_state(state)?;

    Ok(FaultSafeCopyinGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
        good_result: state.good_result,
        bad_result: state.bad_result,
        page_fault: state.page_fault,
        terminal_frame: state.terminal_frame,
        terminal_rsp: state.terminal_rsp,
        terminal_cs: state.terminal_cs,
        terminal_rflags: state.terminal_rflags,
        final_cr2: state.final_cr2,
        efer: state.msrs[0],
        star: state.msrs[1],
        lstar: state.msrs[2],
        sfmask: state.msrs[3],
        good_page_pte: state.good_page_pte,
        fault_handler_pte: state.fault_handler_pte,
        fault_observation_pte: state.fault_observation_pte,
        bad_pd_entry: state.bad_pd_entry,
    })
}

fn configure_copyin_msrs(backend: &KvmBackend, vcpu: &Vcpu) -> Result<[u64; 4], Error> {
    let indices = [MSR_STAR, MSR_LSTAR, MSR_SFMASK];
    let policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &indices)
        .map_err(|error| verification_error("copyin syscall MSR policy", error.to_string()))?;
    let requested = [
        (MSR_STAR, SYSCALL_STAR_VALUE),
        (MSR_LSTAR, SYSCALL_LSTAR_VALUE),
        (MSR_SFMASK, SYSCALL_SFMASK_VALUE),
    ];
    let values = GuestMsrValueSet::from_policy(&policy, &requested)
        .map_err(|error| verification_error("copyin syscall MSR values", error.to_string()))?;

    let efer = vcpu.enable_efer_bits_preserving(EFER_SYSCALL_ENABLE)?;
    vcpu.set_msrs(&values)?;
    let observed = vcpu.msrs(&indices)?;
    if observed.values().len() != indices.len() {
        return Err(verification_error(
            "copyin syscall MSR readback",
            format!(
                "expected {} values, got {}",
                indices.len(),
                observed.values().len()
            ),
        ));
    }
    let readback = [
        observed.values()[0].value(),
        observed.values()[1].value(),
        observed.values()[2].value(),
    ];
    if readback
        != [
            SYSCALL_STAR_VALUE,
            SYSCALL_LSTAR_VALUE,
            SYSCALL_SFMASK_VALUE,
        ]
    {
        return Err(verification_error(
            "copyin syscall MSR readback",
            format!("unexpected readback {readback:#x?}"),
        ));
    }
    Ok([efer, readback[0], readback[1], readback[2]])
}

fn install_page_fault_gate(memory: &mut GuestMemory) -> Result<(), Error> {
    memory.write(
        GuestPhysAddr::new(
            PRIVILEGE_IDT_ADDR.get()
                + u64::from(COPYIN_PAGE_FAULT_VECTOR) * PAGE_FAULT_GATE_SIZE,
        ),
        &encode_kernel_interrupt_gate(COPYIN_PAGE_FAULT_HANDLER.get()),
    )
}

fn encode_kernel_interrupt_gate(handler: u64) -> [u8; 16] {
    let mut gate = [0_u8; 16];
    gate[0..2].copy_from_slice(&(handler as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&KERNEL_CODE_SELECTOR.to_le_bytes());
    gate[4] = 0;
    gate[5] = 0x8e; // present DPL0 64-bit interrupt gate
    gate[6..8].copy_from_slice(&((handler >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((handler >> 32) as u32).to_le_bytes());
    gate
}

fn read_results(memory: &GuestMemory) -> Result<(u64, u64), Error> {
    let mut bytes = [0_u8; 16];
    memory.read(COPYIN_RESULT_ADDR, &mut bytes)?;
    Ok((read_u64(&bytes, 0), read_u64(&bytes, 8)))
}

fn read_page_fault_observation(
    memory: &GuestMemory,
) -> Result<CopyinPageFaultObservation, Error> {
    let mut bytes = [0_u8; PAGE_FAULT_OBSERVATION_BYTES];
    memory.read(COPYIN_FAULT_OBSERVATION_ADDR, &mut bytes)?;
    Ok(CopyinPageFaultObservation {
        cr2: read_u64(&bytes, 0),
        error_code: read_u64(&bytes, 8),
        rip: read_u64(&bytes, 16),
        cs: read_u64(&bytes, 24),
        rflags: read_u64(&bytes, 32),
    })
}

fn read_terminal_frame(memory: &GuestMemory) -> Result<CopyinTerminalFrame, Error> {
    let start = GuestPhysAddr::new(PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES);
    let mut bytes = [0_u8; PRIVILEGE_FRAME_BYTES as usize];
    memory.read(start, &mut bytes)?;
    Ok(CopyinTerminalFrame {
        rip: read_u64(&bytes, 0),
        cs: read_u64(&bytes, 8),
        rflags: read_u64(&bytes, 16),
        rsp: read_u64(&bytes, 24),
        ss: read_u64(&bytes, 32),
    })
}

fn read_pte(memory: &GuestMemory, address: u64) -> Result<u64, Error> {
    let page = address & !(LONG_MODE_PAGE_SIZE - 1);
    let index = page / LONG_MODE_PAGE_SIZE;
    let mut bytes = [0_u8; 8];
    memory.read(
        GuestPhysAddr::new(PRIVILEGE_PT_ADDR.get() + index * 8),
        &mut bytes,
    )?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_bad_pointer_pd_entry(memory: &GuestMemory) -> Result<u64, Error> {
    let mut bytes = [0_u8; 8];
    memory.read(
        GuestPhysAddr::new(LONG_MODE_PD_ADDR.get() + BAD_POINTER_PD_INDEX * 8),
        &mut bytes,
    )?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed copyin observation field remains eight bytes"),
    )
}

fn validate_runtime_state(state: RuntimeState) -> Result<(), Error> {
    let expected_frame = CopyinTerminalFrame {
        rip: COPYIN_TERMINAL_RETURN_RIP,
        cs: u64::from(PRIVILEGE_USER_CODE_SELECTOR),
        rflags: X86_RFLAGS_RESERVED | X86_RFLAGS_IF,
        rsp: PRIVILEGE_USER_STACK,
        ss: u64::from(PRIVILEGE_USER_DATA_SELECTOR),
    };
    if state.good_result != u64::from(COPYIN_GOOD_VALUE)
        || state.bad_result != COPYIN_EFAULT
        || state.page_fault.cr2 != COPYIN_BAD_POINTER
        || state.page_fault.error_code != PAGE_FAULT_ERROR_CODE
        || state.page_fault.rip != COPYIN_FAULT_RIP
        || state.page_fault.cs != u64::from(KERNEL_CODE_SELECTOR)
        || state.page_fault.rflags != PAGE_FAULT_SAVED_RFLAGS
        || state.terminal_frame != expected_frame
        || state.terminal_rsp != PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES
        || state.terminal_cs != KERNEL_CODE_SELECTOR
        || state.terminal_rflags & X86_RFLAGS_RESERVED != X86_RFLAGS_RESERVED
        || state.terminal_rflags & X86_RFLAGS_IF != 0
        || state.final_cr2 != COPYIN_BAD_POINTER
        || state.msrs[0] & EFER_SYSCALL_ENABLE != EFER_SYSCALL_ENABLE
        || state.msrs[1] != SYSCALL_STAR_VALUE
        || state.msrs[2] != SYSCALL_LSTAR_VALUE
        || state.msrs[3] != SYSCALL_SFMASK_VALUE
        || state.good_page_pte & X86_PAGE_USER == 0
        || state.fault_handler_pte & X86_PAGE_USER != 0
        || state.fault_observation_pte & X86_PAGE_USER != 0
        || state.bad_pd_entry & X86_PAGE_PRESENT != 0
    {
        return Err(verification_error(
            "fault-safe copyin architectural state",
            format!(
                "good={:#x} bad={:#x} pf={:?} frame={:?} terminal={:#x}/{:#x}/{:#x} cr2={:#x} msrs={:#x?} ptes={:#x}/{:#x}/{:#x} bad_pd={:#x}",
                state.good_result,
                state.bad_result,
                state.page_fault,
                state.terminal_frame,
                state.terminal_rsp,
                state.terminal_cs,
                state.terminal_rflags,
                state.final_cr2,
                state.msrs,
                state.good_page_pte,
                state.fault_handler_pte,
                state.fault_observation_pte,
                state.bad_pd_entry
            ),
        ));
    }
    Ok(())
}

fn verification_error(operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::GuestMemoryRegion;

    fn layout() -> LongModePrivilegeLayout {
        LongModePrivilegeLayout::new(
            GuestMemoryRegion::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn user_sequence_uses_exact_good_bad_and_terminal_boundaries() {
        assert_eq!(&USER_BYTES[10..12], &[0x0f, 0x05]);
        assert_eq!(COPYIN_FIRST_RETURN_RIP, PRIVILEGE_USER_ENTRY.get() + 12);
        assert_eq!(&USER_BYTES[35..37], &[0x0f, 0x05]);
        assert_eq!(COPYIN_SECOND_RETURN_RIP, PRIVILEGE_USER_ENTRY.get() + 37);
        assert_eq!(&USER_BYTES[41..43], &[0xcd, 0x81]);
        assert_eq!(COPYIN_TERMINAL_RETURN_RIP, PRIVILEGE_USER_ENTRY.get() + 43);
    }

    #[test]
    fn copyin_handler_has_one_fault_site_and_one_fixup() {
        assert_eq!(&COPYIN_HANDLER_BYTES[13..16], &[0x0f, 0xb6, 0x07]);
        assert_eq!(COPYIN_FAULT_RIP, SYSCALL_KERNEL_ENTRY.get() + 13);
        assert_eq!(&COPYIN_HANDLER_BYTES[26..29], &[0x4c, 0x89, 0xd4]);
        assert_eq!(COPYIN_FIXUP_RIP, SYSCALL_KERNEL_ENTRY.get() + 26);
        assert_eq!(&COPYIN_HANDLER_BYTES[29..32], &[0x48, 0x0f, 0x07]);
    }

    #[test]
    fn page_fault_handler_rewrites_saved_rip_discards_error_code_and_iretqs() {
        assert_eq!(&PAGE_FAULT_HANDLER_BYTES[0..3], &[0x0f, 0x20, 0xd0]);
        assert_eq!(
            &PAGE_FAULT_HANDLER_BYTES[62..72],
            &[0x48, 0xb8, 0x1a, 0x20, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(&PAGE_FAULT_HANDLER_BYTES[72..77], &[0x48, 0x89, 0x44, 0x24, 0x08]);
        assert_eq!(&PAGE_FAULT_HANDLER_BYTES[80..84], &[0x48, 0x83, 0xc4, 0x08]);
        assert_eq!(&PAGE_FAULT_HANDLER_BYTES[84..86], &[0x48, 0xcf]);
    }

    #[test]
    fn installs_dpl0_page_fault_gate_without_weakening_user_gates() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let layout = layout();
        layout.install_tables(&mut memory).unwrap();
        install_page_fault_gate(&mut memory).unwrap();

        let mut gate = [0_u8; 16];
        memory
            .read(
                GuestPhysAddr::new(
                    PRIVILEGE_IDT_ADDR.get()
                        + u64::from(COPYIN_PAGE_FAULT_VECTOR) * PAGE_FAULT_GATE_SIZE,
                ),
                &mut gate,
            )
            .unwrap();
        assert_eq!(gate, encode_kernel_interrupt_gate(COPYIN_PAGE_FAULT_HANDLER.get()));
        assert_eq!(gate[5], 0x8e);

        let mut terminal_gate = [0_u8; 16];
        memory
            .read(
                GuestPhysAddr::new(PRIVILEGE_IDT_ADDR.get() + 0x81 * PAGE_FAULT_GATE_SIZE),
                &mut terminal_gate,
            )
            .unwrap();
        assert_eq!(terminal_gate[5], 0xee);
    }

    #[test]
    fn bad_pointer_targets_an_absent_pd_entry_outside_identity_map() {
        assert_eq!(COPYIN_BAD_POINTER, 2 * LONG_MODE_IDENTITY_MAP_SIZE);
        assert_eq!(BAD_POINTER_PD_INDEX, 2);
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let layout = layout();
        layout.install_tables(&mut memory).unwrap();
        assert_eq!(read_bad_pointer_pd_entry(&memory).unwrap() & X86_PAGE_PRESENT, 0);
    }

    #[test]
    fn page_fault_saved_flags_include_architectural_resume_flag() {
        assert_eq!(PAGE_FAULT_SAVED_RFLAGS, 0x1_0002);
        assert_eq!(PAGE_FAULT_ERROR_CODE, 0);
    }
}
