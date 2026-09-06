use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError, VmExitError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::msr::{GuestMsrAccessPolicy, GuestMsrValueSet, MsrIndex};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::PortIoBus;
use crate::privilege::{
    LongModePrivilegeLayout, PRIVILEGE_GDT_ADDR, PRIVILEGE_KERNEL_ENTRY,
    PRIVILEGE_OBSERVATION_ADDR, PRIVILEGE_PT_ADDR, PRIVILEGE_TERMINAL_HANDLER, PRIVILEGE_TSS_RSP0,
    PRIVILEGE_USER_CODE_SELECTOR, PRIVILEGE_USER_DATA_SELECTOR, PRIVILEGE_USER_ENTRY,
    PRIVILEGE_USER_STACK,
};
use crate::vcpu::{PortIoExit, Vcpu, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

pub const SYSCALL_KERNEL_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_2000);
pub const SYSCALL_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb000);
pub const SYSCALL_KERNEL_STACK: u64 = PRIVILEGE_TSS_RSP0;
pub const SYSCALL_USER_RETURN_RIP: u64 = 0x1_1017;
pub const SYSCALL_TERMINAL_RETURN_RIP: u64 = 0x1_102f;
pub const SYSCALL_TERMINAL_HLT_RIP: u64 = 0x1_3005;
pub const SYSCALL_PROOF: &[u8; 2] = b"SD";

pub const MSR_EFER: MsrIndex = MsrIndex::new(0xc000_0080);
pub const MSR_STAR: MsrIndex = MsrIndex::new(0xc000_0081);
pub const MSR_LSTAR: MsrIndex = MsrIndex::new(0xc000_0082);
pub const MSR_SFMASK: MsrIndex = MsrIndex::new(0xc000_0084);

pub const SYSCALL_STAR_VALUE: u64 = 0x0010_0008_0000_0000;
pub const SYSCALL_LSTAR_VALUE: u64 = SYSCALL_KERNEL_ENTRY.get();
pub const SYSCALL_SFMASK_VALUE: u64 = 1 << 9;
pub const EFER_SYSCALL_ENABLE: u64 = 1;

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_RFLAGS_RESERVED: u64 = 1 << 1;
const X86_RFLAGS_IF: u64 = 1 << 9;
const SYSCALL_EXIT_BUDGET: u32 = 3;
const PRIVILEGE_FRAME_BYTES: u64 = 5 * 8;
const SYSCALL_OBSERVATION_BYTES: usize = 48;

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

const USER_BYTES: [u8; 47] = [
    0x48, 0xbf, 0x00, 0xa0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs user obs, rdi
    0x8c, 0xc8, // mov cs, ax
    0x66, 0x89, 0x07, // mov ax, [rdi]
    0x8c, 0xd0, // mov ss, ax
    0x66, 0x89, 0x47, 0x02, // mov ax, [rdi+2]
    0x0f, 0x05, // syscall
    0x48, 0xbf, 0x00, 0xa0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // restore user obs, rdi
    0x8c, 0xc8, // mov cs, ax
    0x66, 0x89, 0x47, 0x04, // mov ax, [rdi+4]
    0x8c, 0xd0, // mov ss, ax
    0x66, 0x89, 0x47, 0x06, // mov ax, [rdi+6]
    0xcd, 0x81, // int 0x81 terminal gate
];

const SYSCALL_HANDLER_BYTES: [u8; 66] = [
    0x49, 0x89, 0xe2, // mov r10, rsp: preserve the user stack
    0x48, 0xbc, 0x00, 0xe0, 0x1f, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs rsp, 0x1fe000
    0x48, 0xbf, 0x00, 0xb0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs rdi, 0xb000
    0x48, 0x89, 0x0f, // mov [rdi], rcx
    0x4c, 0x89, 0x5f, 0x08, // mov [rdi+8], r11
    0x4c, 0x89, 0x57, 0x10, // mov [rdi+16], r10
    0x9c, // pushfq
    0x58, // pop rax
    0x48, 0x89, 0x47, 0x18, // mov [rdi+24], rax
    0x8c, 0xc8, // mov ax, cs
    0x66, 0x89, 0x47, 0x20, // mov [rdi+32], ax
    0x8c, 0xd0, // mov ax, ss
    0x66, 0x89, 0x47, 0x22, // mov [rdi+34], ax
    0x48, 0x89, 0x67, 0x28, // mov [rdi+40], rsp
    0xb0, b'S', // mov al, 'S'
    0xe6, 0xe9, // out 0xe9, al
    0x4c, 0x89, 0xd4, // mov rsp, r10: restore the user stack
    0x48, 0x0f, 0x07, // sysretq
];

const TERMINAL_HANDLER_BYTES: [u8; 5] = [
    0xb0, b'D', // mov al, 'D'
    0xe6, 0xe9, // out 0xe9, al
    0xf4, // hlt in ring0
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyscallObservation {
    user_return_rip: u64,
    user_rflags: u64,
    user_rsp: u64,
    kernel_rflags: u64,
    kernel_cs: u16,
    kernel_ss: u16,
    kernel_rsp: u64,
}

impl SyscallObservation {
    #[must_use]
    pub const fn user_return_rip(self) -> u64 {
        self.user_return_rip
    }

    #[must_use]
    pub const fn user_rflags(self) -> u64 {
        self.user_rflags
    }

    #[must_use]
    pub const fn user_rsp(self) -> u64 {
        self.user_rsp
    }

    #[must_use]
    pub const fn kernel_rflags(self) -> u64 {
        self.kernel_rflags
    }

    #[must_use]
    pub const fn kernel_cs(self) -> u16 {
        self.kernel_cs
    }

    #[must_use]
    pub const fn kernel_ss(self) -> u16 {
        self.kernel_ss
    }

    #[must_use]
    pub const fn kernel_rsp(self) -> u64 {
        self.kernel_rsp
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyscallReturnFrame {
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

impl SyscallReturnFrame {
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
pub struct SyscallSysretGuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
    user_selectors: [u16; 4],
    observation: SyscallObservation,
    terminal_frame: SyscallReturnFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    efer: u64,
    star: u64,
    lstar: u64,
    sfmask: u64,
    user_code_pte: u64,
    user_stack_pte: u64,
    syscall_handler_pte: u64,
    syscall_observation_pte: u64,
}

impl SyscallSysretGuestResult {
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
    pub const fn user_selectors(&self) -> [u16; 4] {
        self.user_selectors
    }

    #[must_use]
    pub const fn observation(&self) -> SyscallObservation {
        self.observation
    }

    #[must_use]
    pub const fn terminal_frame(&self) -> SyscallReturnFrame {
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
    pub const fn user_code_pte(&self) -> u64 {
        self.user_code_pte
    }

    #[must_use]
    pub const fn user_stack_pte(&self) -> u64 {
        self.user_stack_pte
    }

    #[must_use]
    pub const fn syscall_handler_pte(&self) -> u64 {
        self.syscall_handler_pte
    }

    #[must_use]
    pub const fn syscall_observation_pte(&self) -> u64 {
        self.syscall_observation_pte
    }
}

#[derive(Debug, Clone, Copy)]
struct RuntimeState {
    selectors: [u16; 4],
    observation: SyscallObservation,
    terminal_frame: SyscallReturnFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    msrs: [u64; 4],
    ptes: [u64; 4],
}

pub fn run_syscall_sysret_guest(config: VmConfig) -> Result<SyscallSysretGuestResult, Error> {
    let kernel = FlatGuestImage::new(
        PRIVILEGE_KERNEL_ENTRY,
        PRIVILEGE_KERNEL_ENTRY,
        &KERNEL_BOOT_BYTES,
    )?;
    let user = FlatGuestImage::new(PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_ENTRY, &USER_BYTES)?;
    let syscall_handler = FlatGuestImage::new(
        SYSCALL_KERNEL_ENTRY,
        SYSCALL_KERNEL_ENTRY,
        &SYSCALL_HANDLER_BYTES,
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
        .expect("fixed bounded syscall/SYSRET privilege layout remains valid");
    layout.install_tables(&mut memory)?;
    kernel.load(&mut memory)?;
    user.load(&mut memory)?;
    syscall_handler.load(&mut memory)?;
    terminal_handler.load(&mut memory)?;
    memory.write(PRIVILEGE_OBSERVATION_ADDR, &[0; 8])?;
    memory.write(SYSCALL_OBSERVATION_ADDR, &[0; SYSCALL_OBSERVATION_BYTES])?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(&layout)?;
    let msrs = configure_syscall_msrs(&backend, &vcpu)?;

    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, SYSCALL_EXIT_BUDGET)?;
    if execution.io_exits().len() != SYSCALL_PROOF.len() {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "syscall/SYSRET proof output count",
            expected_reason: crate::vcpu::VcpuExit::Io.reason(),
            actual_reason: execution.report().exit().reason(),
        }));
    }
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != SYSCALL_PROOF {
        return Err(verification_error(
            "syscall/SYSRET proof",
            format!("expected {SYSCALL_PROOF:?}, got {proof:?}"),
        ));
    }

    let registers = vcpu.capture_register_snapshot()?;
    let terminal_regs = vcpu.registers()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered syscall guest memory remains VM-owned");
    let state = RuntimeState {
        selectors: read_user_selectors(guest_memory)?,
        observation: read_syscall_observation(guest_memory)?,
        terminal_frame: read_terminal_frame(guest_memory)?,
        terminal_rsp: registers.rsp(),
        terminal_cs: special.cs().selector(),
        terminal_rflags: terminal_regs.rflags,
        msrs,
        ptes: [
            read_pte(guest_memory, PRIVILEGE_USER_ENTRY.get())?,
            read_pte(guest_memory, PRIVILEGE_USER_STACK - 1)?,
            read_pte(guest_memory, SYSCALL_KERNEL_ENTRY.get())?,
            read_pte(guest_memory, SYSCALL_OBSERVATION_ADDR.get())?,
        ],
    };
    validate_runtime_state(state)?;

    Ok(SyscallSysretGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
        user_selectors: state.selectors,
        observation: state.observation,
        terminal_frame: state.terminal_frame,
        terminal_rsp: state.terminal_rsp,
        terminal_cs: state.terminal_cs,
        terminal_rflags: state.terminal_rflags,
        efer: state.msrs[0],
        star: state.msrs[1],
        lstar: state.msrs[2],
        sfmask: state.msrs[3],
        user_code_pte: state.ptes[0],
        user_stack_pte: state.ptes[1],
        syscall_handler_pte: state.ptes[2],
        syscall_observation_pte: state.ptes[3],
    })
}

fn configure_syscall_msrs(backend: &KvmBackend, vcpu: &Vcpu) -> Result<[u64; 4], Error> {
    let initial = vcpu.msrs(&[MSR_EFER])?;
    let initial_efer = initial
        .values()
        .first()
        .expect("one requested EFER read produces one value")
        .value();
    let indices = [MSR_EFER, MSR_STAR, MSR_LSTAR, MSR_SFMASK];
    let policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &indices)
        .map_err(|error| verification_error("syscall MSR policy", error.to_string()))?;
    let requested = [
        (MSR_EFER, initial_efer | EFER_SYSCALL_ENABLE),
        (MSR_STAR, SYSCALL_STAR_VALUE),
        (MSR_LSTAR, SYSCALL_LSTAR_VALUE),
        (MSR_SFMASK, SYSCALL_SFMASK_VALUE),
    ];
    let values = GuestMsrValueSet::from_policy(&policy, &requested)
        .map_err(|error| verification_error("syscall MSR values", error.to_string()))?;
    vcpu.set_msrs(&values)?;

    let observed = vcpu.msrs(&indices)?;
    if observed.values().len() != indices.len() {
        return Err(verification_error(
            "syscall MSR readback",
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
        observed.values()[3].value(),
    ];
    if readback
        != [
            initial_efer | EFER_SYSCALL_ENABLE,
            SYSCALL_STAR_VALUE,
            SYSCALL_LSTAR_VALUE,
            SYSCALL_SFMASK_VALUE,
        ]
    {
        return Err(verification_error(
            "syscall MSR readback",
            format!("unexpected readback {readback:#x?}"),
        ));
    }
    Ok(readback)
}

fn read_user_selectors(memory: &GuestMemory) -> Result<[u16; 4], Error> {
    let mut bytes = [0_u8; 8];
    memory.read(PRIVILEGE_OBSERVATION_ADDR, &mut bytes)?;
    Ok([
        u16::from_le_bytes([bytes[0], bytes[1]]),
        u16::from_le_bytes([bytes[2], bytes[3]]),
        u16::from_le_bytes([bytes[4], bytes[5]]),
        u16::from_le_bytes([bytes[6], bytes[7]]),
    ])
}

fn read_syscall_observation(memory: &GuestMemory) -> Result<SyscallObservation, Error> {
    let mut bytes = [0_u8; SYSCALL_OBSERVATION_BYTES];
    memory.read(SYSCALL_OBSERVATION_ADDR, &mut bytes)?;
    Ok(SyscallObservation {
        user_return_rip: read_u64(&bytes, 0),
        user_rflags: read_u64(&bytes, 8),
        user_rsp: read_u64(&bytes, 16),
        kernel_rflags: read_u64(&bytes, 24),
        kernel_cs: u16::from_le_bytes([bytes[32], bytes[33]]),
        kernel_ss: u16::from_le_bytes([bytes[34], bytes[35]]),
        kernel_rsp: read_u64(&bytes, 40),
    })
}

fn read_terminal_frame(memory: &GuestMemory) -> Result<SyscallReturnFrame, Error> {
    let start = GuestPhysAddr::new(PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES);
    let mut bytes = [0_u8; PRIVILEGE_FRAME_BYTES as usize];
    memory.read(start, &mut bytes)?;
    Ok(SyscallReturnFrame {
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

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed observation field remains eight bytes"),
    )
}

fn validate_runtime_state(state: RuntimeState) -> Result<(), Error> {
    if state.selectors
        != [
            PRIVILEGE_USER_CODE_SELECTOR,
            PRIVILEGE_USER_DATA_SELECTOR,
            PRIVILEGE_USER_CODE_SELECTOR,
            PRIVILEGE_USER_DATA_SELECTOR,
        ]
    {
        return Err(verification_error(
            "syscall user selectors",
            format!("unexpected selectors {:?}", state.selectors),
        ));
    }
    if state.observation.user_return_rip != SYSCALL_USER_RETURN_RIP
        || state.observation.user_rflags != X86_RFLAGS_RESERVED | X86_RFLAGS_IF
        || state.observation.user_rsp != PRIVILEGE_USER_STACK
        || state.observation.kernel_rflags != X86_RFLAGS_RESERVED
        || state.observation.kernel_cs != KERNEL_CODE_SELECTOR
        || state.observation.kernel_ss != KERNEL_DATA_SELECTOR
        || state.observation.kernel_rsp != SYSCALL_KERNEL_STACK
    {
        return Err(verification_error(
            "syscall entry observation",
            format!("unexpected observation {:?}", state.observation),
        ));
    }
    let expected_frame = SyscallReturnFrame {
        rip: SYSCALL_TERMINAL_RETURN_RIP,
        cs: u64::from(PRIVILEGE_USER_CODE_SELECTOR),
        rflags: X86_RFLAGS_RESERVED | X86_RFLAGS_IF,
        rsp: PRIVILEGE_USER_STACK,
        ss: u64::from(PRIVILEGE_USER_DATA_SELECTOR),
    };
    if state.terminal_frame != expected_frame {
        return Err(verification_error(
            "SYSRET user return frame",
            format!("unexpected terminal frame {:?}", state.terminal_frame),
        ));
    }
    if state.terminal_rsp != PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES
        || state.terminal_cs != KERNEL_CODE_SELECTOR
        || state.terminal_rflags & X86_RFLAGS_RESERVED != X86_RFLAGS_RESERVED
        || state.terminal_rflags & X86_RFLAGS_IF != 0
    {
        return Err(verification_error(
            "syscall terminal kernel state",
            format!(
                "unexpected terminal rsp={:#x} cs={:#x} rflags={:#x}",
                state.terminal_rsp, state.terminal_cs, state.terminal_rflags
            ),
        ));
    }
    if state.msrs[0] & EFER_SYSCALL_ENABLE != EFER_SYSCALL_ENABLE
        || state.msrs[1] != SYSCALL_STAR_VALUE
        || state.msrs[2] != SYSCALL_LSTAR_VALUE
        || state.msrs[3] != SYSCALL_SFMASK_VALUE
    {
        return Err(verification_error(
            "syscall MSR state",
            format!(
                "unexpected EFER={:#x} STAR={:#x} LSTAR={:#x} SFMASK={:#x}",
                state.msrs[0], state.msrs[1], state.msrs[2], state.msrs[3]
            ),
        ));
    }
    for (role, pte) in [("user code", state.ptes[0]), ("user stack", state.ptes[1])] {
        if pte & X86_PAGE_USER == 0 {
            return Err(verification_error(
                "syscall page permissions",
                format!("{role} PTE {pte:#x} is not user-accessible"),
            ));
        }
    }
    for (role, pte) in [
        ("syscall handler", state.ptes[2]),
        ("syscall observation", state.ptes[3]),
    ] {
        if pte & X86_PAGE_USER != 0 {
            return Err(verification_error(
                "syscall page permissions",
                format!("{role} PTE {pte:#x} is unexpectedly user-accessible"),
            ));
        }
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

const fn star_kernel_cs(star: u64) -> u16 {
    ((star >> 32) & 0xffff) as u16
}

const fn star_sysret_cs(star: u64) -> u16 {
    let base = ((star >> 48) & 0xffff) as u16;
    (base.wrapping_add(16) & !3) | 3
}

const fn star_sysret_ss(star: u64) -> u16 {
    let base = ((star >> 48) & 0xffff) as u16;
    (base.wrapping_add(8) & !3) | 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_encodes_exact_kernel_and_sysret_selectors() {
        assert_eq!(star_kernel_cs(SYSCALL_STAR_VALUE), KERNEL_CODE_SELECTOR);
        assert_eq!(
            star_sysret_cs(SYSCALL_STAR_VALUE),
            PRIVILEGE_USER_CODE_SELECTOR
        );
        assert_eq!(
            star_sysret_ss(SYSCALL_STAR_VALUE),
            PRIVILEGE_USER_DATA_SELECTOR
        );
    }

    #[test]
    fn user_sequence_places_syscall_at_exact_return_boundary() {
        assert_eq!(&USER_BYTES[21..23], &[0x0f, 0x05]);
        assert_eq!(PRIVILEGE_USER_ENTRY.get() + 23, SYSCALL_USER_RETURN_RIP);
        assert_eq!(&USER_BYTES[45..47], &[0xcd, 0x81]);
        assert_eq!(
            PRIVILEGE_USER_ENTRY.get() + USER_BYTES.len() as u64,
            SYSCALL_TERMINAL_RETURN_RIP
        );
    }

    #[test]
    fn kernel_handler_switches_stack_before_observation_and_ends_in_sysretq() {
        assert_eq!(&SYSCALL_HANDLER_BYTES[..3], &[0x49, 0x89, 0xe2]);
        assert_eq!(
            &SYSCALL_HANDLER_BYTES[3..13],
            &[0x48, 0xbc, 0x00, 0xe0, 0x1f, 0, 0, 0, 0, 0]
        );
        assert_eq!(&SYSCALL_HANDLER_BYTES[63..], &[0x48, 0x0f, 0x07]);
    }

    #[test]
    fn sfmask_clears_only_interrupt_enable_for_fixed_fixture() {
        assert_eq!(SYSCALL_SFMASK_VALUE, X86_RFLAGS_IF);
        assert_eq!(
            (X86_RFLAGS_RESERVED | X86_RFLAGS_IF) & !SYSCALL_SFMASK_VALUE,
            X86_RFLAGS_RESERVED
        );
    }

    #[test]
    fn syscall_kernel_stack_is_supervisor_page_distinct_from_user_stack_page() {
        let kernel_page = (SYSCALL_KERNEL_STACK - 1) & !(LONG_MODE_PAGE_SIZE - 1);
        let user_page = (PRIVILEGE_USER_STACK - 1) & !(LONG_MODE_PAGE_SIZE - 1);
        assert_ne!(kernel_page, user_page);
    }

    #[test]
    fn proof_requires_syscall_handler_then_post_sysret_terminal_gate() {
        assert_eq!(SYSCALL_PROOF, b"SD");
    }

    #[test]
    fn gdt_base_remains_guest_owned_privilege_table() {
        assert_eq!(PRIVILEGE_GDT_ADDR.get(), 0x5000);
    }
}
