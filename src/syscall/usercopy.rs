use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError, VmExitError};
use crate::execution::run_vcpu_until_stopped;
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
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

use super::{
    configure_syscall_msrs, EFER_SYSCALL_ENABLE, SYSCALL_KERNEL_ENTRY, SYSCALL_LSTAR_VALUE,
    SYSCALL_SFMASK_VALUE, SYSCALL_STAR_VALUE,
};

pub const USERCOPY_SOURCE: u64 = 0xa100;
pub const USERCOPY_DESTINATION: u64 = 0xa101;
pub const USERCOPY_RESULT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xa180);
pub const USERCOPY_READ_FAULT_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb000);
pub const USERCOPY_WRITE_FAULT_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb040);
pub const USERCOPY_FIXUP_TABLE_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb100);
pub const USERCOPY_BAD_POINTER: u64 = 0x40_0000;
pub const USERCOPY_PAGE_FAULT_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_4000);
pub const USERCOPY_PAGE_FAULT_VECTOR: u8 = 14;
pub const USERCOPY_READ_FAULT_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 13;
pub const USERCOPY_WRITE_FAULT_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 16;
pub const USERCOPY_READ_FIXUP_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 27;
pub const USERCOPY_WRITE_FIXUP_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 33;
pub const USERCOPY_COMMON_RETURN_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 37;
pub const USERCOPY_TERMINAL_RETURN_RIP: u64 = PRIVILEGE_USER_ENTRY.get() + 96;
pub const USERCOPY_TERMINAL_HLT_RIP: u64 = PRIVILEGE_TERMINAL_HANDLER.get() + 5;
pub const USERCOPY_VALUE: u8 = 0x6b;
pub const USERCOPY_EFAULT: u64 = (-14_i64) as u64;
pub const USERCOPY_PROOF: &[u8; 4] = b"CRWD";

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_RFLAGS_RESERVED: u64 = 1 << 1;
const X86_RFLAGS_IF: u64 = 1 << 9;
const X86_RFLAGS_RF: u64 = 1 << 16;
const USERCOPY_EXIT_BUDGET: u32 = 5;
const PRIVILEGE_FRAME_BYTES: u64 = 5 * 8;
const FAULT_OBSERVATION_BYTES: usize = 48;
const FIXUP_ENTRY_BYTES: usize = 24;
const FIXUP_TABLE_BYTES: usize = 2 * FIXUP_ENTRY_BYTES;
const PAGE_FAULT_GATE_SIZE: u64 = 16;
const READ_PAGE_FAULT_ERROR_CODE: u64 = 0;
const WRITE_PAGE_FAULT_ERROR_CODE: u64 = 1 << 1;
const PAGE_FAULT_SAVED_RFLAGS: u64 = X86_RFLAGS_RESERVED | X86_RFLAGS_RF;
const BAD_POINTER_PD_INDEX: u64 = (USERCOPY_BAD_POINTER >> 21) & 0x1ff;

const KERNEL_BOOT_BYTES: [u8; 41] = [
    0xfa, 0x66, 0xb8, 0x28, 0x00, 0x0f, 0x00, 0xd8, 0x6a, 0x1b, 0x48, 0xb8, 0x00, 0xd0, 0x1f, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x50, 0x68, 0x02, 0x02, 0x00, 0x00, 0x6a, 0x23, 0x48, 0xb8, 0x00, 0x10,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x48, 0xcf,
];

const USER_BYTES: [u8; 96] = [
    0x48, 0xbf, 0x00, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xbe, 0x01, 0xa1, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x48, 0xbb, 0x80, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x48, 0x89, 0x03, 0x0f, 0xb6, 0x06, 0x48, 0x89, 0x43, 0x18, 0x48, 0xbf, 0x00, 0x00, 0x40, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x48, 0xbe, 0x01, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x05,
    0x48, 0x89, 0x43, 0x08, 0x48, 0xbf, 0x00, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xbe,
    0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x48, 0x89, 0x43, 0x10, 0xcd, 0x81,
];

const USERCOPY_HANDLER_BYTES: [u8; 46] = [
    0x49, 0x89, 0xe2, 0x48, 0xbc, 0x00, 0xe0, 0x1f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0xb6, 0x07,
    0x88, 0x06, 0x45, 0x31, 0xc0, 0xb0, b'C', 0xe6, 0xe9, 0xeb, 0x0a, 0xb0, b'R', 0xe6, 0xe9, 0xeb,
    0x04, 0xb0, b'W', 0xe6, 0xe9, 0x4c, 0x89, 0xc0, 0x4c, 0x89, 0xd4, 0x48, 0x0f, 0x07,
];

const PAGE_FAULT_HANDLER_BYTES: [u8; 112] = [
    0x48, 0x8b, 0x44, 0x24, 0x08, 0x49, 0xb9, 0x00, 0xb1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49,
    0x3b, 0x01, 0x74, 0x0b, 0x49, 0x3b, 0x41, 0x18, 0x74, 0x0f, 0xb0, b'X', 0xe6, 0xe9, 0xf4, 0x4d,
    0x8b, 0x41, 0x08, 0x49, 0x8b, 0x51, 0x10, 0xeb, 0x08, 0x4d, 0x8b, 0x41, 0x20, 0x49, 0x8b, 0x51,
    0x28, 0x0f, 0x20, 0xd0, 0x48, 0x89, 0x02, 0x48, 0x8b, 0x04, 0x24, 0x48, 0x89, 0x42, 0x08, 0x48,
    0x8b, 0x44, 0x24, 0x08, 0x48, 0x89, 0x42, 0x10, 0x48, 0x8b, 0x44, 0x24, 0x10, 0x48, 0x89, 0x42,
    0x18, 0x48, 0x8b, 0x44, 0x24, 0x18, 0x48, 0x89, 0x42, 0x20, 0x4c, 0x89, 0x42, 0x28, 0x4c, 0x89,
    0x44, 0x24, 0x08, 0x49, 0xc7, 0xc0, 0xf2, 0xff, 0xff, 0xff, 0x48, 0x83, 0xc4, 0x08, 0x48, 0xcf,
];

const TERMINAL_HANDLER_BYTES: [u8; 5] = [0xb0, b'D', 0xe6, 0xe9, 0xf4];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsercopyFixupEntry {
    fault_rip: u64,
    fixup_rip: u64,
    observation_addr: u64,
}

impl UsercopyFixupEntry {
    #[must_use]
    pub const fn fault_rip(self) -> u64 {
        self.fault_rip
    }
    #[must_use]
    pub const fn fixup_rip(self) -> u64 {
        self.fixup_rip
    }
    #[must_use]
    pub const fn observation_addr(self) -> u64 {
        self.observation_addr
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsercopyFaultObservation {
    cr2: u64,
    error_code: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    resolved_fixup: u64,
}

impl UsercopyFaultObservation {
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
    #[must_use]
    pub const fn resolved_fixup(self) -> u64 {
        self.resolved_fixup
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsercopyTerminalFrame {
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

impl UsercopyTerminalFrame {
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
pub struct FaultSafeUsercopyGuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
    good_return: u64,
    bad_source_return: u64,
    bad_destination_return: u64,
    user_readback: u64,
    source_value: u8,
    destination_value: u8,
    read_fault: UsercopyFaultObservation,
    write_fault: UsercopyFaultObservation,
    fixup_entries: [UsercopyFixupEntry; 2],
    terminal_frame: UsercopyTerminalFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    final_cr2: u64,
    msrs: [u64; 4],
    user_page_pte: u64,
    fault_handler_pte: u64,
    fault_metadata_pte: u64,
    bad_pd_entry: u64,
}

impl FaultSafeUsercopyGuestResult {
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
    pub const fn good_return(&self) -> u64 {
        self.good_return
    }
    #[must_use]
    pub const fn bad_source_return(&self) -> u64 {
        self.bad_source_return
    }
    #[must_use]
    pub const fn bad_destination_return(&self) -> u64 {
        self.bad_destination_return
    }
    #[must_use]
    pub const fn user_readback(&self) -> u64 {
        self.user_readback
    }
    #[must_use]
    pub const fn source_value(&self) -> u8 {
        self.source_value
    }
    #[must_use]
    pub const fn destination_value(&self) -> u8 {
        self.destination_value
    }
    #[must_use]
    pub const fn read_fault(&self) -> UsercopyFaultObservation {
        self.read_fault
    }
    #[must_use]
    pub const fn write_fault(&self) -> UsercopyFaultObservation {
        self.write_fault
    }
    #[must_use]
    pub const fn fixup_entries(&self) -> &[UsercopyFixupEntry; 2] {
        &self.fixup_entries
    }
    #[must_use]
    pub const fn terminal_frame(&self) -> UsercopyTerminalFrame {
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
        self.msrs[0]
    }
    #[must_use]
    pub const fn star(&self) -> u64 {
        self.msrs[1]
    }
    #[must_use]
    pub const fn lstar(&self) -> u64 {
        self.msrs[2]
    }
    #[must_use]
    pub const fn sfmask(&self) -> u64 {
        self.msrs[3]
    }
    #[must_use]
    pub const fn user_page_pte(&self) -> u64 {
        self.user_page_pte
    }
    #[must_use]
    pub const fn fault_handler_pte(&self) -> u64 {
        self.fault_handler_pte
    }
    #[must_use]
    pub const fn fault_metadata_pte(&self) -> u64 {
        self.fault_metadata_pte
    }
    #[must_use]
    pub const fn bad_pd_entry(&self) -> u64 {
        self.bad_pd_entry
    }
}

#[derive(Debug, Clone, Copy)]
struct RuntimeState {
    good_return: u64,
    bad_source_return: u64,
    bad_destination_return: u64,
    user_readback: u64,
    source_value: u8,
    destination_value: u8,
    read_fault: UsercopyFaultObservation,
    write_fault: UsercopyFaultObservation,
    fixup_entries: [UsercopyFixupEntry; 2],
    terminal_frame: UsercopyTerminalFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    final_cr2: u64,
    msrs: [u64; 4],
    user_page_pte: u64,
    fault_handler_pte: u64,
    fault_metadata_pte: u64,
    bad_pd_entry: u64,
}

pub fn run_fault_safe_usercopy_guest(
    config: VmConfig,
) -> Result<FaultSafeUsercopyGuestResult, Error> {
    let kernel = FlatGuestImage::new(
        PRIVILEGE_KERNEL_ENTRY,
        PRIVILEGE_KERNEL_ENTRY,
        &KERNEL_BOOT_BYTES,
    )?;
    let user = FlatGuestImage::new(PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_ENTRY, &USER_BYTES)?;
    let service = FlatGuestImage::new(
        SYSCALL_KERNEL_ENTRY,
        SYSCALL_KERNEL_ENTRY,
        &USERCOPY_HANDLER_BYTES,
    )?;
    let page_fault_handler = FlatGuestImage::new(
        USERCOPY_PAGE_FAULT_HANDLER,
        USERCOPY_PAGE_FAULT_HANDLER,
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
        .expect("fixed bounded fault-safe usercopy privilege layout remains valid");
    layout.install_tables(&mut memory)?;
    install_page_fault_gate(&mut memory)?;
    kernel.load(&mut memory)?;
    user.load(&mut memory)?;
    service.load(&mut memory)?;
    page_fault_handler.load(&mut memory)?;
    terminal_handler.load(&mut memory)?;
    memory.write(GuestPhysAddr::new(USERCOPY_SOURCE), &[USERCOPY_VALUE, 0])?;
    memory.write(USERCOPY_RESULT_ADDR, &[0; 32])?;
    memory.write(
        USERCOPY_READ_FAULT_OBSERVATION_ADDR,
        &[0; FAULT_OBSERVATION_BYTES],
    )?;
    memory.write(
        USERCOPY_WRITE_FAULT_OBSERVATION_ADDR,
        &[0; FAULT_OBSERVATION_BYTES],
    )?;
    memory.write(USERCOPY_FIXUP_TABLE_ADDR, &encoded_fixup_table())?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(&layout)?;
    let msrs = configure_syscall_msrs(&backend, &vcpu)?;

    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, USERCOPY_EXIT_BUDGET)?;
    if execution.io_exits().len() != USERCOPY_PROOF.len() {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "fault-safe usercopy proof output count",
            expected_reason: VcpuExit::Io.reason(),
            actual_reason: execution.report().exit().reason(),
        }));
    }
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != USERCOPY_PROOF {
        return Err(verification_error(
            "fault-safe usercopy proof",
            format!("expected {USERCOPY_PROOF:?}, got {proof:?}"),
        ));
    }
    for (io_exit, expected) in execution
        .io_exits()
        .iter()
        .zip(USERCOPY_PROOF.iter().copied())
    {
        if io_exit.direction() != PortIoDirection::Out
            || io_exit.size() != 1
            || io_exit.count() != 1
            || io_exit.output_data() != [expected]
        {
            return Err(verification_error(
                "fault-safe usercopy port I/O metadata",
                format!("unexpected exit {io_exit:?} for byte {expected:#x}"),
            ));
        }
    }

    let registers = vcpu.capture_register_snapshot()?;
    let terminal_regs = vcpu.registers()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered usercopy guest memory remains VM-owned");
    let (good_return, bad_source_return, bad_destination_return, user_readback) =
        read_results(guest_memory)?;
    let state = RuntimeState {
        good_return,
        bad_source_return,
        bad_destination_return,
        user_readback,
        source_value: read_byte(guest_memory, USERCOPY_SOURCE)?,
        destination_value: read_byte(guest_memory, USERCOPY_DESTINATION)?,
        read_fault: read_fault_observation(guest_memory, USERCOPY_READ_FAULT_OBSERVATION_ADDR)?,
        write_fault: read_fault_observation(guest_memory, USERCOPY_WRITE_FAULT_OBSERVATION_ADDR)?,
        fixup_entries: read_fixup_table(guest_memory)?,
        terminal_frame: read_terminal_frame(guest_memory)?,
        terminal_rsp: registers.rsp(),
        terminal_cs: special.cs().selector(),
        terminal_rflags: terminal_regs.rflags,
        final_cr2: special.cr2(),
        msrs,
        user_page_pte: read_pte(guest_memory, USERCOPY_SOURCE)?,
        fault_handler_pte: read_pte(guest_memory, USERCOPY_PAGE_FAULT_HANDLER.get())?,
        fault_metadata_pte: read_pte(guest_memory, USERCOPY_READ_FAULT_OBSERVATION_ADDR.get())?,
        bad_pd_entry: read_bad_pointer_pd_entry(guest_memory)?,
    };
    validate_runtime_state(state, execution.report())?;

    Ok(FaultSafeUsercopyGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
        good_return: state.good_return,
        bad_source_return: state.bad_source_return,
        bad_destination_return: state.bad_destination_return,
        user_readback: state.user_readback,
        source_value: state.source_value,
        destination_value: state.destination_value,
        read_fault: state.read_fault,
        write_fault: state.write_fault,
        fixup_entries: state.fixup_entries,
        terminal_frame: state.terminal_frame,
        terminal_rsp: state.terminal_rsp,
        terminal_cs: state.terminal_cs,
        terminal_rflags: state.terminal_rflags,
        final_cr2: state.final_cr2,
        msrs: state.msrs,
        user_page_pte: state.user_page_pte,
        fault_handler_pte: state.fault_handler_pte,
        fault_metadata_pte: state.fault_metadata_pte,
        bad_pd_entry: state.bad_pd_entry,
    })
}

fn install_page_fault_gate(memory: &mut GuestMemory) -> Result<(), Error> {
    memory.write(
        GuestPhysAddr::new(
            PRIVILEGE_IDT_ADDR.get() + u64::from(USERCOPY_PAGE_FAULT_VECTOR) * PAGE_FAULT_GATE_SIZE,
        ),
        &encode_kernel_interrupt_gate(USERCOPY_PAGE_FAULT_HANDLER.get()),
    )
}

fn encode_kernel_interrupt_gate(handler: u64) -> [u8; 16] {
    let mut gate = [0_u8; 16];
    gate[0..2].copy_from_slice(&(handler as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&KERNEL_CODE_SELECTOR.to_le_bytes());
    gate[5] = 0x8e;
    gate[6..8].copy_from_slice(&((handler >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((handler >> 32) as u32).to_le_bytes());
    gate
}

fn expected_fixup_entries() -> [UsercopyFixupEntry; 2] {
    [
        UsercopyFixupEntry {
            fault_rip: USERCOPY_READ_FAULT_RIP,
            fixup_rip: USERCOPY_READ_FIXUP_RIP,
            observation_addr: USERCOPY_READ_FAULT_OBSERVATION_ADDR.get(),
        },
        UsercopyFixupEntry {
            fault_rip: USERCOPY_WRITE_FAULT_RIP,
            fixup_rip: USERCOPY_WRITE_FIXUP_RIP,
            observation_addr: USERCOPY_WRITE_FAULT_OBSERVATION_ADDR.get(),
        },
    ]
}

fn encoded_fixup_table() -> [u8; FIXUP_TABLE_BYTES] {
    let mut bytes = [0_u8; FIXUP_TABLE_BYTES];
    for (index, entry) in expected_fixup_entries().iter().copied().enumerate() {
        let offset = index * FIXUP_ENTRY_BYTES;
        bytes[offset..offset + 8].copy_from_slice(&entry.fault_rip.to_le_bytes());
        bytes[offset + 8..offset + 16].copy_from_slice(&entry.fixup_rip.to_le_bytes());
        bytes[offset + 16..offset + 24].copy_from_slice(&entry.observation_addr.to_le_bytes());
    }
    bytes
}

fn read_results(memory: &GuestMemory) -> Result<(u64, u64, u64, u64), Error> {
    let mut bytes = [0_u8; 32];
    memory.read(USERCOPY_RESULT_ADDR, &mut bytes)?;
    Ok((
        read_u64(&bytes, 0),
        read_u64(&bytes, 8),
        read_u64(&bytes, 16),
        read_u64(&bytes, 24),
    ))
}

fn read_byte(memory: &GuestMemory, address: u64) -> Result<u8, Error> {
    let mut byte = [0_u8; 1];
    memory.read(GuestPhysAddr::new(address), &mut byte)?;
    Ok(byte[0])
}

fn read_fault_observation(
    memory: &GuestMemory,
    address: GuestPhysAddr,
) -> Result<UsercopyFaultObservation, Error> {
    let mut bytes = [0_u8; FAULT_OBSERVATION_BYTES];
    memory.read(address, &mut bytes)?;
    Ok(UsercopyFaultObservation {
        cr2: read_u64(&bytes, 0),
        error_code: read_u64(&bytes, 8),
        rip: read_u64(&bytes, 16),
        cs: read_u64(&bytes, 24),
        rflags: read_u64(&bytes, 32),
        resolved_fixup: read_u64(&bytes, 40),
    })
}

fn read_fixup_table(memory: &GuestMemory) -> Result<[UsercopyFixupEntry; 2], Error> {
    let mut bytes = [0_u8; FIXUP_TABLE_BYTES];
    memory.read(USERCOPY_FIXUP_TABLE_ADDR, &mut bytes)?;
    let mut entries = [UsercopyFixupEntry {
        fault_rip: 0,
        fixup_rip: 0,
        observation_addr: 0,
    }; 2];
    for (index, entry) in entries.iter_mut().enumerate() {
        let offset = index * FIXUP_ENTRY_BYTES;
        *entry = UsercopyFixupEntry {
            fault_rip: read_u64(&bytes, offset),
            fixup_rip: read_u64(&bytes, offset + 8),
            observation_addr: read_u64(&bytes, offset + 16),
        };
    }
    Ok(entries)
}

fn read_terminal_frame(memory: &GuestMemory) -> Result<UsercopyTerminalFrame, Error> {
    let start = GuestPhysAddr::new(PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES);
    let mut bytes = [0_u8; PRIVILEGE_FRAME_BYTES as usize];
    memory.read(start, &mut bytes)?;
    Ok(UsercopyTerminalFrame {
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
            .expect("fixed usercopy field is eight bytes"),
    )
}

fn validate_runtime_state(state: RuntimeState, report: VmExitReport) -> Result<(), Error> {
    let expected_read_fault = UsercopyFaultObservation {
        cr2: USERCOPY_BAD_POINTER,
        error_code: READ_PAGE_FAULT_ERROR_CODE,
        rip: USERCOPY_READ_FAULT_RIP,
        cs: u64::from(KERNEL_CODE_SELECTOR),
        rflags: PAGE_FAULT_SAVED_RFLAGS,
        resolved_fixup: USERCOPY_READ_FIXUP_RIP,
    };
    let expected_write_fault = UsercopyFaultObservation {
        cr2: USERCOPY_BAD_POINTER,
        error_code: WRITE_PAGE_FAULT_ERROR_CODE,
        rip: USERCOPY_WRITE_FAULT_RIP,
        cs: u64::from(KERNEL_CODE_SELECTOR),
        rflags: PAGE_FAULT_SAVED_RFLAGS,
        resolved_fixup: USERCOPY_WRITE_FIXUP_RIP,
    };
    let expected_frame = UsercopyTerminalFrame {
        rip: USERCOPY_TERMINAL_RETURN_RIP,
        cs: u64::from(PRIVILEGE_USER_CODE_SELECTOR),
        rflags: X86_RFLAGS_RESERVED | X86_RFLAGS_IF,
        rsp: PRIVILEGE_USER_STACK,
        ss: u64::from(PRIVILEGE_USER_DATA_SELECTOR),
    };
    if state.good_return != 0
        || state.bad_source_return != USERCOPY_EFAULT
        || state.bad_destination_return != USERCOPY_EFAULT
        || state.user_readback != u64::from(USERCOPY_VALUE)
        || state.source_value != USERCOPY_VALUE
        || state.destination_value != USERCOPY_VALUE
        || state.read_fault != expected_read_fault
        || state.write_fault != expected_write_fault
        || state.fixup_entries != expected_fixup_entries()
        || state.terminal_frame != expected_frame
        || state.terminal_rsp != PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES
        || state.terminal_cs != KERNEL_CODE_SELECTOR
        || state.terminal_rflags & X86_RFLAGS_RESERVED != X86_RFLAGS_RESERVED
        || state.terminal_rflags & X86_RFLAGS_IF != 0
        || state.final_cr2 != USERCOPY_BAD_POINTER
        || state.msrs[0] & EFER_SYSCALL_ENABLE != EFER_SYSCALL_ENABLE
        || state.msrs[1] != SYSCALL_STAR_VALUE
        || state.msrs[2] != SYSCALL_LSTAR_VALUE
        || state.msrs[3] != SYSCALL_SFMASK_VALUE
        || state.user_page_pte & (X86_PAGE_USER | X86_PAGE_WRITE)
            != (X86_PAGE_USER | X86_PAGE_WRITE)
        || state.fault_handler_pte & X86_PAGE_USER != 0
        || state.fault_metadata_pte & X86_PAGE_USER != 0
        || state.bad_pd_entry & X86_PAGE_PRESENT != 0
        || report.exit() != VcpuExit::Hlt
        || report.rip() != USERCOPY_TERMINAL_HLT_RIP
        || report.rflags() & X86_RFLAGS_RESERVED != X86_RFLAGS_RESERVED
    {
        return Err(verification_error(
            "fault-safe usercopy architectural state",
            format!(
                "returns={:#x}/{:#x}/{:#x} readback={:#x} bytes={:#x}/{:#x} read_pf={:?} write_pf={:?} table={:?} frame={:?} terminal={:#x}/{:#x}/{:#x} cr2={:#x} msrs={:#x?} ptes={:#x}/{:#x}/{:#x} bad_pd={:#x} report={:?}",
                state.good_return,
                state.bad_source_return,
                state.bad_destination_return,
                state.user_readback,
                state.source_value,
                state.destination_value,
                state.read_fault,
                state.write_fault,
                state.fixup_entries,
                state.terminal_frame,
                state.terminal_rsp,
                state.terminal_cs,
                state.terminal_rflags,
                state.final_cr2,
                state.msrs,
                state.user_page_pte,
                state.fault_handler_pte,
                state.fault_metadata_pte,
                state.bad_pd_entry,
                report
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
    fn service_has_two_distinct_fault_sites_and_two_fixups() {
        assert_eq!(&USERCOPY_HANDLER_BYTES[13..16], &[0x0f, 0xb6, 0x07]);
        assert_eq!(&USERCOPY_HANDLER_BYTES[16..18], &[0x88, 0x06]);
        assert_eq!(USERCOPY_READ_FAULT_RIP, 0x1200d);
        assert_eq!(USERCOPY_WRITE_FAULT_RIP, 0x12010);
        assert_eq!(&USERCOPY_HANDLER_BYTES[27..31], &[0xb0, b'R', 0xe6, 0xe9]);
        assert_eq!(&USERCOPY_HANDLER_BYTES[33..37], &[0xb0, b'W', 0xe6, 0xe9]);
        assert_eq!(USERCOPY_READ_FIXUP_RIP, 0x1201b);
        assert_eq!(USERCOPY_WRITE_FIXUP_RIP, 0x12021);
        assert_eq!(USERCOPY_COMMON_RETURN_RIP, 0x12025);
    }

    #[test]
    fn guest_sequence_runs_good_bad_source_bad_destination_then_terminal() {
        assert_eq!(&USER_BYTES[20..22], &[0x0f, 0x05]);
        assert_eq!(&USER_BYTES[62..64], &[0x0f, 0x05]);
        assert_eq!(&USER_BYTES[90..92], &[0x0f, 0x05]);
        assert_eq!(&USER_BYTES[94..96], &[0xcd, 0x81]);
        assert_eq!(USERCOPY_TERMINAL_RETURN_RIP, 0x11060);
    }

    #[test]
    fn fixup_table_encodes_two_supervisor_recovery_entries() {
        let expected = expected_fixup_entries();
        let bytes = encoded_fixup_table();
        assert_eq!(bytes.len(), 48);
        assert_eq!(read_u64(&bytes, 0), expected[0].fault_rip());
        assert_eq!(read_u64(&bytes, 8), expected[0].fixup_rip());
        assert_eq!(read_u64(&bytes, 16), expected[0].observation_addr());
        assert_eq!(read_u64(&bytes, 24), expected[1].fault_rip());
        assert_eq!(read_u64(&bytes, 32), expected[1].fixup_rip());
        assert_eq!(read_u64(&bytes, 40), expected[1].observation_addr());
    }

    #[test]
    fn page_fault_handler_matches_only_table_sites_and_fails_closed_otherwise() {
        assert_eq!(
            &PAGE_FAULT_HANDLER_BYTES[15..20],
            &[0x49, 0x3b, 0x01, 0x74, 0x0b]
        );
        assert_eq!(
            &PAGE_FAULT_HANDLER_BYTES[20..26],
            &[0x49, 0x3b, 0x41, 0x18, 0x74, 0x0f]
        );
        assert_eq!(
            &PAGE_FAULT_HANDLER_BYTES[26..31],
            &[0xb0, b'X', 0xe6, 0xe9, 0xf4]
        );
        assert_eq!(
            &PAGE_FAULT_HANDLER_BYTES[106..110],
            &[0x48, 0x83, 0xc4, 0x08]
        );
        assert_eq!(&PAGE_FAULT_HANDLER_BYTES[110..112], &[0x48, 0xcf]);
    }

    #[test]
    fn page_fault_gate_is_dpl0_and_terminal_gate_remains_dpl3() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let layout = layout();
        layout.install_tables(&mut memory).unwrap();
        install_page_fault_gate(&mut memory).unwrap();
        let mut gate = [0_u8; 16];
        memory
            .read(
                GuestPhysAddr::new(
                    PRIVILEGE_IDT_ADDR.get() + u64::from(USERCOPY_PAGE_FAULT_VECTOR) * 16,
                ),
                &mut gate,
            )
            .unwrap();
        assert_eq!(gate[5], 0x8e);
        let mut terminal_gate = [0_u8; 16];
        memory
            .read(
                GuestPhysAddr::new(PRIVILEGE_IDT_ADDR.get() + 0x81 * 16),
                &mut terminal_gate,
            )
            .unwrap();
        assert_eq!(terminal_gate[5], 0xee);
    }

    #[test]
    fn user_data_is_writable_metadata_is_supervisor_and_bad_pointer_is_unmapped() {
        assert_eq!(USERCOPY_BAD_POINTER, 2 * LONG_MODE_IDENTITY_MAP_SIZE);
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let layout = layout();
        layout.install_tables(&mut memory).unwrap();
        let user_pte = read_pte(&memory, USERCOPY_SOURCE).unwrap();
        let metadata_pte = read_pte(&memory, USERCOPY_FIXUP_TABLE_ADDR.get()).unwrap();
        assert_eq!(
            user_pte & (X86_PAGE_USER | X86_PAGE_WRITE),
            X86_PAGE_USER | X86_PAGE_WRITE
        );
        assert_eq!(metadata_pte & X86_PAGE_USER, 0);
        assert_eq!(
            read_bad_pointer_pd_entry(&memory).unwrap() & X86_PAGE_PRESENT,
            0
        );
    }

    #[test]
    fn fault_contract_distinguishes_read_and_write_page_faults() {
        assert_eq!(READ_PAGE_FAULT_ERROR_CODE, 0);
        assert_eq!(WRITE_PAGE_FAULT_ERROR_CODE, 2);
        assert_eq!(PAGE_FAULT_SAVED_RFLAGS, 0x1_0002);
    }
}
