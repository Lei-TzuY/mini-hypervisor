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
use crate::vcpu::{PortIoDirection, PortIoExit, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

use super::{
    configure_syscall_msrs, EFER_SYSCALL_ENABLE, SYSCALL_KERNEL_ENTRY, SYSCALL_LSTAR_VALUE,
    SYSCALL_SFMASK_VALUE, SYSCALL_STAR_VALUE,
};

pub const DISPATCH_COPY_NR: u64 = 0;
pub const DISPATCH_PUTC_NR: u64 = 1;
pub const DISPATCH_UNKNOWN_NR: u64 = 0xff;
pub const DISPATCH_SOURCE: u64 = 0xa100;
pub const DISPATCH_DESTINATION: u64 = 0xa101;
pub const DISPATCH_RESULT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xa180);
pub const DISPATCH_READ_FAULT_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb000);
pub const DISPATCH_WRITE_FAULT_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb040);
pub const DISPATCH_FIXUP_TABLE_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb100);
pub const DISPATCH_BAD_POINTER: u64 = 0x40_0000;
pub const DISPATCH_PAGE_FAULT_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_4000);
pub const DISPATCH_PAGE_FAULT_VECTOR: u8 = 14;
pub const DISPATCH_COPY_READ_FAULT_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 38;
pub const DISPATCH_COPY_WRITE_FAULT_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 41;
pub const DISPATCH_COPY_READ_FIXUP_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 51;
pub const DISPATCH_COPY_WRITE_FIXUP_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 64;
pub const DISPATCH_COMMON_RETURN_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 84;
pub const DISPATCH_TERMINAL_RETURN_RIP: u64 = PRIVILEGE_USER_ENTRY.get() + 138;
pub const DISPATCH_TERMINAL_HLT_RIP: u64 = PRIVILEGE_TERMINAL_HANDLER.get() + 5;
pub const DISPATCH_VALUE: u8 = 0x6b;
pub const DISPATCH_PUTC_VALUE: u8 = b'P';
pub const DISPATCH_EFAULT: u64 = (-14_i64) as u64;
pub const DISPATCH_ENOSYS: u64 = (-38_i64) as u64;
pub const DISPATCH_PROOF: &[u8; 6] = b"CPRWUD";

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_RFLAGS_RESERVED: u64 = 1 << 1;
const X86_RFLAGS_PF: u64 = 1 << 2;
const X86_RFLAGS_ZF: u64 = 1 << 6;
const X86_RFLAGS_IF: u64 = 1 << 9;
const X86_RFLAGS_RF: u64 = 1 << 16;
const DISPATCH_EXIT_BUDGET: u32 = 7;
const PRIVILEGE_FRAME_BYTES: u64 = 5 * 8;
const FAULT_OBSERVATION_BYTES: usize = 48;
const FIXUP_ENTRY_BYTES: usize = 24;
const FIXUP_TABLE_BYTES: usize = 2 * FIXUP_ENTRY_BYTES;
const PAGE_FAULT_GATE_SIZE: u64 = 16;
const READ_PAGE_FAULT_ERROR_CODE: u64 = 0;
const WRITE_PAGE_FAULT_ERROR_CODE: u64 = 1 << 1;
const PAGE_FAULT_SAVED_RFLAGS: u64 =
    X86_RFLAGS_RESERVED | X86_RFLAGS_PF | X86_RFLAGS_ZF | X86_RFLAGS_RF;
const BAD_POINTER_PD_INDEX: u64 = (DISPATCH_BAD_POINTER >> 21) & 0x1ff;

const KERNEL_BOOT_BYTES: [u8; 41] = [
    0xfa, 0x66, 0xb8, 0x28, 0x00, 0x0f, 0x00, 0xd8, 0x6a, 0x1b, 0x48, 0xb8, 0x00, 0xd0, 0x1f, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x50, 0x68, 0x02, 0x02, 0x00, 0x00, 0x6a, 0x23, 0x48, 0xb8, 0x00, 0x10,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x48, 0xcf,
];

const USER_BYTES: [u8; 138] = [
    0xb8, 0x00, 0x00, 0x00, 0x00, 0x48, 0xbf, 0x00, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48,
    0xbe, 0x01, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x48, 0xbb, 0x80, 0xa1, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x89, 0x03, 0x0f, 0xb6, 0x06, 0x48, 0x89, 0x43, 0x08, 0xb8,
    0x01, 0x00, 0x00, 0x00, 0xbf, 0x50, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x48, 0x89, 0x43, 0x10, 0xb8,
    0x00, 0x00, 0x00, 0x00, 0x48, 0xbf, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xbe,
    0x01, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x48, 0x89, 0x43, 0x18, 0xb8, 0x00,
    0x00, 0x00, 0x00, 0x48, 0xbf, 0x00, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xbe, 0x00,
    0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x48, 0x89, 0x43, 0x20, 0xb8, 0xff, 0x00,
    0x00, 0x00, 0x0f, 0x05, 0x48, 0x89, 0x43, 0x28, 0xcd, 0x81,
];

const DISPATCHER_HANDLER_BYTES: [u8; 90] = [
    0x49, 0x89, 0xe2, 0x48, 0xbc, 0x00, 0xe0, 0x1f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x83, 0xf8,
    0x00, 0x74, 0x13, 0x48, 0x83, 0xf8, 0x01, 0x74, 0x34, 0xb0, b'U', 0xe6, 0xe9, 0x48, 0xc7, 0xc0,
    0xda, 0xff, 0xff, 0xff, 0xeb, 0x2e, 0x0f, 0xb6, 0x07, 0x88, 0x06, 0xb0, b'C', 0xe6, 0xe9, 0x31,
    0xc0, 0xeb, 0x21, 0xb0, b'R', 0xe6, 0xe9, 0x48, 0xc7, 0xc0, 0xf2, 0xff, 0xff, 0xff, 0xeb, 0x14,
    0xb0, b'W', 0xe6, 0xe9, 0x48, 0xc7, 0xc0, 0xf2, 0xff, 0xff, 0xff, 0xeb, 0x07, 0x40, 0x88, 0xf8,
    0xe6, 0xe9, 0x31, 0xc0, 0x4c, 0x89, 0xd4, 0x48, 0x0f, 0x07,
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
pub struct DispatchFixupEntry {
    fault_rip: u64,
    fixup_rip: u64,
    observation_addr: u64,
}

impl DispatchFixupEntry {
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
pub struct DispatchFaultObservation {
    cr2: u64,
    error_code: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    resolved_fixup: u64,
}

impl DispatchFaultObservation {
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
pub struct DispatchTerminalFrame {
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

impl DispatchTerminalFrame {
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
pub struct SyscallDispatchGuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
    copy_return: u64,
    copy_readback: u64,
    putc_return: u64,
    bad_source_return: u64,
    bad_destination_return: u64,
    unknown_return: u64,
    source_value: u8,
    destination_value: u8,
    read_fault: DispatchFaultObservation,
    write_fault: DispatchFaultObservation,
    fixup_entries: [DispatchFixupEntry; 2],
    terminal_frame: DispatchTerminalFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    final_cr2: u64,
    msrs: [u64; 4],
    user_page_pte: u64,
    dispatcher_pte: u64,
    fault_handler_pte: u64,
    fault_metadata_pte: u64,
    bad_pd_entry: u64,
}

impl SyscallDispatchGuestResult {
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
    pub const fn copy_return(&self) -> u64 {
        self.copy_return
    }

    #[must_use]
    pub const fn copy_readback(&self) -> u64 {
        self.copy_readback
    }

    #[must_use]
    pub const fn putc_return(&self) -> u64 {
        self.putc_return
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
    pub const fn unknown_return(&self) -> u64 {
        self.unknown_return
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
    pub const fn read_fault(&self) -> DispatchFaultObservation {
        self.read_fault
    }

    #[must_use]
    pub const fn write_fault(&self) -> DispatchFaultObservation {
        self.write_fault
    }

    #[must_use]
    pub const fn fixup_entries(&self) -> &[DispatchFixupEntry; 2] {
        &self.fixup_entries
    }

    #[must_use]
    pub const fn terminal_frame(&self) -> DispatchTerminalFrame {
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
    pub const fn dispatcher_pte(&self) -> u64 {
        self.dispatcher_pte
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
    copy_return: u64,
    copy_readback: u64,
    putc_return: u64,
    bad_source_return: u64,
    bad_destination_return: u64,
    unknown_return: u64,
    source_value: u8,
    destination_value: u8,
    read_fault: DispatchFaultObservation,
    write_fault: DispatchFaultObservation,
    fixup_entries: [DispatchFixupEntry; 2],
    terminal_frame: DispatchTerminalFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    final_cr2: u64,
    msrs: [u64; 4],
    user_page_pte: u64,
    dispatcher_pte: u64,
    fault_handler_pte: u64,
    fault_metadata_pte: u64,
    bad_pd_entry: u64,
}

pub fn run_syscall_dispatch_guest(config: VmConfig) -> Result<SyscallDispatchGuestResult, Error> {
    let kernel = FlatGuestImage::new(
        PRIVILEGE_KERNEL_ENTRY,
        PRIVILEGE_KERNEL_ENTRY,
        &KERNEL_BOOT_BYTES,
    )?;
    let user = FlatGuestImage::new(PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_ENTRY, &USER_BYTES)?;
    let dispatcher = FlatGuestImage::new(
        SYSCALL_KERNEL_ENTRY,
        SYSCALL_KERNEL_ENTRY,
        &DISPATCHER_HANDLER_BYTES,
    )?;
    let page_fault_handler = FlatGuestImage::new(
        DISPATCH_PAGE_FAULT_HANDLER,
        DISPATCH_PAGE_FAULT_HANDLER,
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
        .expect("fixed bounded syscall dispatcher privilege layout remains valid");
    layout.install_tables(&mut memory)?;
    install_page_fault_gate(&mut memory)?;
    kernel.load(&mut memory)?;
    user.load(&mut memory)?;
    dispatcher.load(&mut memory)?;
    page_fault_handler.load(&mut memory)?;
    terminal_handler.load(&mut memory)?;
    memory.write(GuestPhysAddr::new(DISPATCH_SOURCE), &[DISPATCH_VALUE, 0])?;
    memory.write(DISPATCH_RESULT_ADDR, &[0; 48])?;
    memory.write(
        DISPATCH_READ_FAULT_OBSERVATION_ADDR,
        &[0; FAULT_OBSERVATION_BYTES],
    )?;
    memory.write(
        DISPATCH_WRITE_FAULT_OBSERVATION_ADDR,
        &[0; FAULT_OBSERVATION_BYTES],
    )?;
    memory.write(DISPATCH_FIXUP_TABLE_ADDR, &encoded_fixup_table())?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(&layout)?;
    let msrs = configure_syscall_msrs(&backend, &vcpu)?;

    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, DISPATCH_EXIT_BUDGET)?;
    if execution.io_exits().len() != DISPATCH_PROOF.len() {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "syscall dispatcher proof output count",
            expected_reason: VcpuExit::Io.reason(),
            actual_reason: execution.report().exit().reason(),
        }));
    }
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != DISPATCH_PROOF {
        return Err(verification_error(
            "syscall dispatcher proof",
            format!("expected {DISPATCH_PROOF:?}, got {proof:?}"),
        ));
    }
    for (io_exit, expected) in execution
        .io_exits()
        .iter()
        .zip(DISPATCH_PROOF.iter().copied())
    {
        if io_exit.direction() != PortIoDirection::Out
            || io_exit.size() != 1
            || io_exit.count() != 1
            || io_exit.output_data() != [expected]
        {
            return Err(verification_error(
                "syscall dispatcher port I/O metadata",
                format!("unexpected exit {io_exit:?} for byte {expected:#x}"),
            ));
        }
    }

    let registers = vcpu.capture_register_snapshot()?;
    let terminal_regs = vcpu.registers()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered dispatcher guest memory remains VM-owned");
    let results = read_results(guest_memory)?;
    let state = RuntimeState {
        copy_return: results[0],
        copy_readback: results[1],
        putc_return: results[2],
        bad_source_return: results[3],
        bad_destination_return: results[4],
        unknown_return: results[5],
        source_value: read_byte(guest_memory, DISPATCH_SOURCE)?,
        destination_value: read_byte(guest_memory, DISPATCH_DESTINATION)?,
        read_fault: read_fault_observation(guest_memory, DISPATCH_READ_FAULT_OBSERVATION_ADDR)?,
        write_fault: read_fault_observation(guest_memory, DISPATCH_WRITE_FAULT_OBSERVATION_ADDR)?,
        fixup_entries: read_fixup_table(guest_memory)?,
        terminal_frame: read_terminal_frame(guest_memory)?,
        terminal_rsp: registers.rsp(),
        terminal_cs: special.cs().selector(),
        terminal_rflags: terminal_regs.rflags,
        final_cr2: special.cr2(),
        msrs,
        user_page_pte: read_pte(guest_memory, DISPATCH_SOURCE)?,
        dispatcher_pte: read_pte(guest_memory, SYSCALL_KERNEL_ENTRY.get())?,
        fault_handler_pte: read_pte(guest_memory, DISPATCH_PAGE_FAULT_HANDLER.get())?,
        fault_metadata_pte: read_pte(guest_memory, DISPATCH_READ_FAULT_OBSERVATION_ADDR.get())?,
        bad_pd_entry: read_bad_pointer_pd_entry(guest_memory)?,
    };
    validate_runtime_state(state, execution.report())?;

    Ok(SyscallDispatchGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
        copy_return: state.copy_return,
        copy_readback: state.copy_readback,
        putc_return: state.putc_return,
        bad_source_return: state.bad_source_return,
        bad_destination_return: state.bad_destination_return,
        unknown_return: state.unknown_return,
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
        dispatcher_pte: state.dispatcher_pte,
        fault_handler_pte: state.fault_handler_pte,
        fault_metadata_pte: state.fault_metadata_pte,
        bad_pd_entry: state.bad_pd_entry,
    })
}

fn install_page_fault_gate(memory: &mut GuestMemory) -> Result<(), Error> {
    memory.write(
        GuestPhysAddr::new(
            PRIVILEGE_IDT_ADDR.get() + u64::from(DISPATCH_PAGE_FAULT_VECTOR) * PAGE_FAULT_GATE_SIZE,
        ),
        &encode_kernel_interrupt_gate(DISPATCH_PAGE_FAULT_HANDLER.get()),
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

fn expected_fixup_entries() -> [DispatchFixupEntry; 2] {
    [
        DispatchFixupEntry {
            fault_rip: DISPATCH_COPY_READ_FAULT_RIP,
            fixup_rip: DISPATCH_COPY_READ_FIXUP_RIP,
            observation_addr: DISPATCH_READ_FAULT_OBSERVATION_ADDR.get(),
        },
        DispatchFixupEntry {
            fault_rip: DISPATCH_COPY_WRITE_FAULT_RIP,
            fixup_rip: DISPATCH_COPY_WRITE_FIXUP_RIP,
            observation_addr: DISPATCH_WRITE_FAULT_OBSERVATION_ADDR.get(),
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

fn read_results(memory: &GuestMemory) -> Result<[u64; 6], Error> {
    let mut bytes = [0_u8; 48];
    memory.read(DISPATCH_RESULT_ADDR, &mut bytes)?;
    Ok([
        read_u64(&bytes, 0),
        read_u64(&bytes, 8),
        read_u64(&bytes, 16),
        read_u64(&bytes, 24),
        read_u64(&bytes, 32),
        read_u64(&bytes, 40),
    ])
}

fn read_byte(memory: &GuestMemory, address: u64) -> Result<u8, Error> {
    let mut byte = [0_u8; 1];
    memory.read(GuestPhysAddr::new(address), &mut byte)?;
    Ok(byte[0])
}

fn read_fault_observation(
    memory: &GuestMemory,
    address: GuestPhysAddr,
) -> Result<DispatchFaultObservation, Error> {
    let mut bytes = [0_u8; FAULT_OBSERVATION_BYTES];
    memory.read(address, &mut bytes)?;
    Ok(DispatchFaultObservation {
        cr2: read_u64(&bytes, 0),
        error_code: read_u64(&bytes, 8),
        rip: read_u64(&bytes, 16),
        cs: read_u64(&bytes, 24),
        rflags: read_u64(&bytes, 32),
        resolved_fixup: read_u64(&bytes, 40),
    })
}

fn read_fixup_table(memory: &GuestMemory) -> Result<[DispatchFixupEntry; 2], Error> {
    let mut bytes = [0_u8; FIXUP_TABLE_BYTES];
    memory.read(DISPATCH_FIXUP_TABLE_ADDR, &mut bytes)?;
    let mut entries = [DispatchFixupEntry {
        fault_rip: 0,
        fixup_rip: 0,
        observation_addr: 0,
    }; 2];
    for (index, entry) in entries.iter_mut().enumerate() {
        let offset = index * FIXUP_ENTRY_BYTES;
        *entry = DispatchFixupEntry {
            fault_rip: read_u64(&bytes, offset),
            fixup_rip: read_u64(&bytes, offset + 8),
            observation_addr: read_u64(&bytes, offset + 16),
        };
    }
    Ok(entries)
}

fn read_terminal_frame(memory: &GuestMemory) -> Result<DispatchTerminalFrame, Error> {
    let start = GuestPhysAddr::new(PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES);
    let mut bytes = [0_u8; PRIVILEGE_FRAME_BYTES as usize];
    memory.read(start, &mut bytes)?;
    Ok(DispatchTerminalFrame {
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
            .expect("fixed dispatcher field is eight bytes"),
    )
}

fn validate_runtime_state(state: RuntimeState, report: VmExitReport) -> Result<(), Error> {
    let expected_read_fault = DispatchFaultObservation {
        cr2: DISPATCH_BAD_POINTER,
        error_code: READ_PAGE_FAULT_ERROR_CODE,
        rip: DISPATCH_COPY_READ_FAULT_RIP,
        cs: u64::from(KERNEL_CODE_SELECTOR),
        rflags: PAGE_FAULT_SAVED_RFLAGS,
        resolved_fixup: DISPATCH_COPY_READ_FIXUP_RIP,
    };
    let expected_write_fault = DispatchFaultObservation {
        cr2: DISPATCH_BAD_POINTER,
        error_code: WRITE_PAGE_FAULT_ERROR_CODE,
        rip: DISPATCH_COPY_WRITE_FAULT_RIP,
        cs: u64::from(KERNEL_CODE_SELECTOR),
        rflags: PAGE_FAULT_SAVED_RFLAGS,
        resolved_fixup: DISPATCH_COPY_WRITE_FIXUP_RIP,
    };
    let expected_frame = DispatchTerminalFrame {
        rip: DISPATCH_TERMINAL_RETURN_RIP,
        cs: u64::from(PRIVILEGE_USER_CODE_SELECTOR),
        rflags: X86_RFLAGS_RESERVED | X86_RFLAGS_IF,
        rsp: PRIVILEGE_USER_STACK,
        ss: u64::from(PRIVILEGE_USER_DATA_SELECTOR),
    };

    if state.copy_return != 0
        || state.copy_readback != u64::from(DISPATCH_VALUE)
        || state.putc_return != 0
        || state.bad_source_return != DISPATCH_EFAULT
        || state.bad_destination_return != DISPATCH_EFAULT
        || state.unknown_return != DISPATCH_ENOSYS
        || state.source_value != DISPATCH_VALUE
        || state.destination_value != DISPATCH_VALUE
        || state.read_fault != expected_read_fault
        || state.write_fault != expected_write_fault
        || state.fixup_entries != expected_fixup_entries()
        || state.terminal_frame != expected_frame
        || state.terminal_rsp != PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES
        || state.terminal_cs != KERNEL_CODE_SELECTOR
        || state.terminal_rflags & X86_RFLAGS_RESERVED != X86_RFLAGS_RESERVED
        || state.terminal_rflags & X86_RFLAGS_IF != 0
        || state.final_cr2 != DISPATCH_BAD_POINTER
        || state.msrs[0] & EFER_SYSCALL_ENABLE != EFER_SYSCALL_ENABLE
        || state.msrs[1] != SYSCALL_STAR_VALUE
        || state.msrs[2] != SYSCALL_LSTAR_VALUE
        || state.msrs[3] != SYSCALL_SFMASK_VALUE
        || state.user_page_pte & (X86_PAGE_USER | X86_PAGE_WRITE)
            != (X86_PAGE_USER | X86_PAGE_WRITE)
        || state.dispatcher_pte & X86_PAGE_USER != 0
        || state.fault_handler_pte & X86_PAGE_USER != 0
        || state.fault_metadata_pte & X86_PAGE_USER != 0
        || state.bad_pd_entry & X86_PAGE_PRESENT != 0
        || report.exit() != VcpuExit::Hlt
        || report.rip() != DISPATCH_TERMINAL_HLT_RIP
        || report.rflags() & X86_RFLAGS_RESERVED != X86_RFLAGS_RESERVED
    {
        return Err(verification_error(
            "syscall dispatcher architectural state",
            format!(
                "returns={:#x}/{:#x}/{:#x}/{:#x}/{:#x} readback={:#x} bytes={:#x}/{:#x} read_pf={:?} write_pf={:?} table={:?} frame={:?} terminal={:#x}/{:#x}/{:#x} cr2={:#x} msrs={:#x?} ptes={:#x}/{:#x}/{:#x}/{:#x} bad_pd={:#x} report={:?}",
                state.copy_return,
                state.putc_return,
                state.bad_source_return,
                state.bad_destination_return,
                state.unknown_return,
                state.copy_readback,
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
                state.dispatcher_pte,
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
    fn dispatcher_uses_rax_number_and_two_distinct_service_paths() {
        assert_eq!(
            &DISPATCHER_HANDLER_BYTES[13..19],
            &[0x48, 0x83, 0xf8, 0x00, 0x74, 0x13]
        );
        assert_eq!(
            &DISPATCHER_HANDLER_BYTES[19..25],
            &[0x48, 0x83, 0xf8, 0x01, 0x74, 0x34]
        );
        assert_eq!(&DISPATCHER_HANDLER_BYTES[25..29], &[0xb0, b'U', 0xe6, 0xe9]);
        assert_eq!(
            &DISPATCHER_HANDLER_BYTES[38..43],
            &[0x0f, 0xb6, 0x07, 0x88, 0x06]
        );
        assert_eq!(
            &DISPATCHER_HANDLER_BYTES[77..84],
            &[0x40, 0x88, 0xf8, 0xe6, 0xe9, 0x31, 0xc0]
        );
        assert_eq!(DISPATCH_COPY_NR, 0);
        assert_eq!(DISPATCH_PUTC_NR, 1);
        assert_eq!(DISPATCH_UNKNOWN_NR, 0xff);
        assert_eq!(PAGE_FAULT_SAVED_RFLAGS, 0x10046);
    }

    #[test]
    fn copy_service_has_two_exact_fault_sites_and_fixups() {
        assert_eq!(DISPATCH_COPY_READ_FAULT_RIP, 0x12026);
        assert_eq!(DISPATCH_COPY_WRITE_FAULT_RIP, 0x12029);
        assert_eq!(DISPATCH_COPY_READ_FIXUP_RIP, 0x12033);
        assert_eq!(DISPATCH_COPY_WRITE_FIXUP_RIP, 0x12040);
        assert_eq!(DISPATCH_COMMON_RETURN_RIP, 0x12054);
        assert_eq!(&DISPATCHER_HANDLER_BYTES[51..55], &[0xb0, b'R', 0xe6, 0xe9]);
        assert_eq!(&DISPATCHER_HANDLER_BYTES[64..68], &[0xb0, b'W', 0xe6, 0xe9]);
    }

    #[test]
    fn ring3_program_uses_five_syscalls_and_no_direct_debug_port_output() {
        let syscall_count = USER_BYTES
            .windows(2)
            .filter(|window| *window == [0x0f, 0x05])
            .count();
        let direct_debug_out = USER_BYTES.windows(2).any(|window| window == [0xe6, 0xe9]);
        assert_eq!(syscall_count, 5);
        assert!(!direct_debug_out);
        assert_eq!(&USER_BYTES[136..138], &[0xcd, 0x81]);
        assert_eq!(DISPATCH_TERMINAL_RETURN_RIP, 0x1108a);
    }

    #[test]
    fn fixup_table_encodes_only_the_two_copy_fault_sites() {
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
    fn page_fault_handler_fails_closed_for_unlisted_dispatch_faults() {
        assert_eq!(
            &PAGE_FAULT_HANDLER_BYTES[26..31],
            &[0xb0, b'X', 0xe6, 0xe9, 0xf4]
        );
        assert_eq!(
            &PAGE_FAULT_HANDLER_BYTES[106..112],
            &[0x48, 0x83, 0xc4, 0x08, 0x48, 0xcf]
        );
    }

    #[test]
    fn dispatcher_data_is_user_writable_but_handlers_and_metadata_are_supervisor_only() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let layout = layout();
        layout.install_tables(&mut memory).unwrap();
        install_page_fault_gate(&mut memory).unwrap();
        let user_pte = read_pte(&memory, DISPATCH_SOURCE).unwrap();
        let dispatcher_pte = read_pte(&memory, SYSCALL_KERNEL_ENTRY.get()).unwrap();
        let handler_pte = read_pte(&memory, DISPATCH_PAGE_FAULT_HANDLER.get()).unwrap();
        let metadata_pte = read_pte(&memory, DISPATCH_FIXUP_TABLE_ADDR.get()).unwrap();
        assert_eq!(
            user_pte & (X86_PAGE_USER | X86_PAGE_WRITE),
            X86_PAGE_USER | X86_PAGE_WRITE
        );
        assert_eq!(dispatcher_pte & X86_PAGE_USER, 0);
        assert_eq!(handler_pte & X86_PAGE_USER, 0);
        assert_eq!(metadata_pte & X86_PAGE_USER, 0);
        assert_eq!(
            read_bad_pointer_pd_entry(&memory).unwrap() & X86_PAGE_PRESENT,
            0
        );
    }
}
