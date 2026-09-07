use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError, VmExitError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::PortIoBus;
use crate::privilege::{
    LongModePrivilegeLayout, PRIVILEGE_IDT_ADDR, PRIVILEGE_KERNEL_ENTRY, PRIVILEGE_PT_ADDR,
    PRIVILEGE_RETURN_HANDLER, PRIVILEGE_TERMINAL_HANDLER, PRIVILEGE_TSS_RSP0,
    PRIVILEGE_USER_CODE_SELECTOR, PRIVILEGE_USER_DATA_SELECTOR, PRIVILEGE_USER_ENTRY,
    PRIVILEGE_USER_STACK,
};
use crate::vcpu::{PortIoDirection, PortIoExit, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

use super::{
    configure_syscall_msrs, EFER_SYSCALL_ENABLE, SYSCALL_KERNEL_ENTRY, SYSCALL_LSTAR_VALUE,
    SYSCALL_SFMASK_VALUE, SYSCALL_STAR_VALUE,
};

pub const CROSS_PAGE_COPY_LEN: u64 = 4;
pub const CROSS_PAGE_GOOD_SOURCE: u64 = 0x20ffe;
pub const CROSS_PAGE_GOOD_DESTINATION: u64 = 0x22ffe;
pub const CROSS_PAGE_SOURCE_FAULT_SOURCE: u64 = 0x24ffe;
pub const CROSS_PAGE_SOURCE_FAULT_DESTINATION: u64 = 0x26ffe;
pub const CROSS_PAGE_DEST_FAULT_SOURCE: u64 = 0x28ffe;
pub const CROSS_PAGE_DEST_FAULT_DESTINATION: u64 = 0x2affe;
pub const CROSS_PAGE_SOURCE_FAULT_ADDR: u64 = 0x25000;
pub const CROSS_PAGE_DEST_FAULT_ADDR: u64 = 0x2b000;
pub const CROSS_PAGE_RESULT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xa180);
pub const CROSS_PAGE_READ_FAULT_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb000);
pub const CROSS_PAGE_WRITE_FAULT_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb040);
pub const CROSS_PAGE_FIXUP_TABLE_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb100);
pub const CROSS_PAGE_PAGE_FAULT_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_4000);
pub const CROSS_PAGE_PAGE_FAULT_VECTOR: u8 = 14;
pub const CROSS_PAGE_READ_FAULT_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 19;
pub const CROSS_PAGE_WRITE_FAULT_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 24;
pub const CROSS_PAGE_COMMON_FIXUP_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 37;
pub const CROSS_PAGE_PROOF: &[u8; 4] = b"KKKD";
pub const CROSS_PAGE_GOOD_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
pub const CROSS_PAGE_SOURCE_FAULT_BYTES: [u8; 4] = [0x55, 0x66, 0x77, 0x88];
pub const CROSS_PAGE_DEST_FAULT_BYTES: [u8; 4] = [0x99, 0xaa, 0xbb, 0xcc];

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_RFLAGS_RESERVED: u64 = 1 << 1;
const X86_RFLAGS_PF: u64 = 1 << 2;
const X86_RFLAGS_ZF: u64 = 1 << 6;
const X86_RFLAGS_IF: u64 = 1 << 9;
const X86_RFLAGS_RF: u64 = 1 << 16;
const CROSS_PAGE_EXIT_BUDGET: u32 = 5;
const PRIVILEGE_FRAME_BYTES: u64 = 5 * 8;
const FAULT_OBSERVATION_BYTES: usize = 48;
const FIXUP_ENTRY_BYTES: usize = 24;
const FIXUP_TABLE_BYTES: usize = 2 * FIXUP_ENTRY_BYTES;
const PAGE_FAULT_GATE_SIZE: u64 = 16;
const PAGE_FAULT_SAVED_RFLAGS: u64 =
    X86_RFLAGS_RESERVED | X86_RFLAGS_PF | X86_RFLAGS_ZF | X86_RFLAGS_RF;

const KERNEL_BOOT_BYTES: [u8; 41] = [
    0xfa, 0x66, 0xb8, 0x28, 0x00, 0x0f, 0x00, 0xd8, 0x6a, 0x1b, 0x48, 0xb8, 0x00, 0xd0, 0x1f, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x50, 0x68, 0x02, 0x02, 0x00, 0x00, 0x6a, 0x23, 0x48, 0xb8, 0x00, 0x10,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x48, 0xcf,
];

const COPY_SERVICE_BYTES: [u8; 46] = [
    0x49, 0x89, 0xe2, 0x48, 0xbc, 0x00, 0xe0, 0x1f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x45, 0x31, 0xc0,
    0x4d, 0x39, 0xc0, 0x42, 0x0f, 0xb6, 0x04, 0x07, 0x42, 0x88, 0x04, 0x06, 0x49, 0xff, 0xc0, 0x49,
    0x83, 0xf8, 0x04, 0x75, 0xeb, 0x4c, 0x89, 0xc0, 0x4c, 0x89, 0xd4, 0x48, 0x0f, 0x07,
];

const PAGE_FAULT_HANDLER_BYTES: [u8; 105] = [
    0x48, 0x8b, 0x44, 0x24, 0x08, 0x49, 0xb9, 0x00, 0xb1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49,
    0x3b, 0x01, 0x74, 0x0b, 0x49, 0x3b, 0x41, 0x18, 0x74, 0x0f, 0xb0, b'X', 0xe6, 0xe9, 0xf4, 0x4d,
    0x8b, 0x61, 0x08, 0x49, 0x8b, 0x51, 0x10, 0xeb, 0x08, 0x4d, 0x8b, 0x61, 0x20, 0x49, 0x8b, 0x51,
    0x28, 0x0f, 0x20, 0xd0, 0x48, 0x89, 0x02, 0x48, 0x8b, 0x04, 0x24, 0x48, 0x89, 0x42, 0x08, 0x48,
    0x8b, 0x44, 0x24, 0x08, 0x48, 0x89, 0x42, 0x10, 0x48, 0x8b, 0x44, 0x24, 0x10, 0x48, 0x89, 0x42,
    0x18, 0x48, 0x8b, 0x44, 0x24, 0x18, 0x48, 0x89, 0x42, 0x20, 0x4c, 0x89, 0x62, 0x28, 0x4c, 0x89,
    0x64, 0x24, 0x08, 0x48, 0x83, 0xc4, 0x08, 0x48, 0xcf,
];

const RETURN_HANDLER_BYTES: [u8; 6] = [0xb0, b'K', 0xe6, 0xe9, 0x48, 0xcf];
const TERMINAL_HANDLER_BYTES: [u8; 5] = [0xb0, b'D', 0xe6, 0xe9, 0xf4];

const USER_PAGES: [(u64, bool); 12] = [
    (0x20000, true),
    (0x21000, true),
    (0x22000, true),
    (0x23000, true),
    (0x24000, true),
    (0x25000, false),
    (0x26000, true),
    (0x27000, true),
    (0x28000, true),
    (0x29000, true),
    (0x2a000, true),
    (0x2b000, false),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrossPageFixupEntry {
    fault_rip: u64,
    fixup_rip: u64,
    observation_addr: u64,
}

impl CrossPageFixupEntry {
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
pub struct CrossPageFaultObservation {
    cr2: u64,
    error_code: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    resolved_fixup: u64,
}

impl CrossPageFaultObservation {
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
pub struct CrossPageTerminalFrame {
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

impl CrossPageTerminalFrame {
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
pub struct CrossPageUsercopyGuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
    returns: [u64; 3],
    good_source: [u8; 4],
    good_destination: [u8; 4],
    source_fault_source: [u8; 4],
    source_fault_destination: [u8; 4],
    destination_fault_source: [u8; 4],
    destination_fault_destination: [u8; 4],
    read_fault: CrossPageFaultObservation,
    write_fault: CrossPageFaultObservation,
    fixup_entries: [CrossPageFixupEntry; 2],
    terminal_frame: CrossPageTerminalFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    final_cr2: u64,
    msrs: [u64; 4],
    user_page_ptes: Vec<(u64, u64)>,
    service_pte: u64,
    fault_handler_pte: u64,
    fault_metadata_pte: u64,
}

impl CrossPageUsercopyGuestResult {
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
    pub const fn returns(&self) -> [u64; 3] {
        self.returns
    }

    #[must_use]
    pub const fn good_source(&self) -> [u8; 4] {
        self.good_source
    }

    #[must_use]
    pub const fn good_destination(&self) -> [u8; 4] {
        self.good_destination
    }

    #[must_use]
    pub const fn source_fault_source(&self) -> [u8; 4] {
        self.source_fault_source
    }

    #[must_use]
    pub const fn source_fault_destination(&self) -> [u8; 4] {
        self.source_fault_destination
    }

    #[must_use]
    pub const fn destination_fault_source(&self) -> [u8; 4] {
        self.destination_fault_source
    }

    #[must_use]
    pub const fn destination_fault_destination(&self) -> [u8; 4] {
        self.destination_fault_destination
    }

    #[must_use]
    pub const fn read_fault(&self) -> CrossPageFaultObservation {
        self.read_fault
    }

    #[must_use]
    pub const fn write_fault(&self) -> CrossPageFaultObservation {
        self.write_fault
    }

    #[must_use]
    pub const fn fixup_entries(&self) -> &[CrossPageFixupEntry; 2] {
        &self.fixup_entries
    }

    #[must_use]
    pub const fn terminal_frame(&self) -> CrossPageTerminalFrame {
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
    pub const fn msrs(&self) -> [u64; 4] {
        self.msrs
    }

    #[must_use]
    pub fn user_page_ptes(&self) -> &[(u64, u64)] {
        &self.user_page_ptes
    }

    #[must_use]
    pub const fn service_pte(&self) -> u64 {
        self.service_pte
    }

    #[must_use]
    pub const fn fault_handler_pte(&self) -> u64 {
        self.fault_handler_pte
    }

    #[must_use]
    pub const fn fault_metadata_pte(&self) -> u64 {
        self.fault_metadata_pte
    }
}

pub fn run_cross_page_usercopy_guest(
    config: VmConfig,
) -> Result<CrossPageUsercopyGuestResult, Error> {
    let user_bytes = build_user_guest();
    let terminal_return_rip = PRIVILEGE_USER_ENTRY.get()
        + u64::try_from(user_bytes.len()).expect("fixed cross-page user program length fits u64");
    let kernel = FlatGuestImage::new(
        PRIVILEGE_KERNEL_ENTRY,
        PRIVILEGE_KERNEL_ENTRY,
        &KERNEL_BOOT_BYTES,
    )?;
    let user = FlatGuestImage::new(PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_ENTRY, &user_bytes)?;
    let service = FlatGuestImage::new(
        SYSCALL_KERNEL_ENTRY,
        SYSCALL_KERNEL_ENTRY,
        &COPY_SERVICE_BYTES,
    )?;
    let page_fault_handler = FlatGuestImage::new(
        CROSS_PAGE_PAGE_FAULT_HANDLER,
        CROSS_PAGE_PAGE_FAULT_HANDLER,
        &PAGE_FAULT_HANDLER_BYTES,
    )?;
    let return_handler = FlatGuestImage::new(
        PRIVILEGE_RETURN_HANDLER,
        PRIVILEGE_RETURN_HANDLER,
        &RETURN_HANDLER_BYTES,
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
        .expect("fixed bounded cross-page usercopy privilege layout remains valid");
    layout.install_tables(&mut memory)?;
    install_cross_page_mappings(&mut memory)?;
    install_page_fault_gate(&mut memory)?;
    kernel.load(&mut memory)?;
    user.load(&mut memory)?;
    service.load(&mut memory)?;
    page_fault_handler.load(&mut memory)?;
    return_handler.load(&mut memory)?;
    terminal_handler.load(&mut memory)?;
    initialize_data(&mut memory)?;
    memory.write(CROSS_PAGE_RESULT_ADDR, &[0; 24])?;
    memory.write(
        CROSS_PAGE_READ_FAULT_OBSERVATION_ADDR,
        &[0; FAULT_OBSERVATION_BYTES],
    )?;
    memory.write(
        CROSS_PAGE_WRITE_FAULT_OBSERVATION_ADDR,
        &[0; FAULT_OBSERVATION_BYTES],
    )?;
    memory.write(CROSS_PAGE_FIXUP_TABLE_ADDR, &encoded_fixup_table())?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(&layout)?;
    let msrs = configure_syscall_msrs(&backend, &vcpu)?;

    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, CROSS_PAGE_EXIT_BUDGET)?;
    if execution.io_exits().len() != CROSS_PAGE_PROOF.len() {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "cross-page usercopy proof output count",
            expected_reason: VcpuExit::Io.reason(),
            actual_reason: execution.report().exit().reason(),
        }));
    }
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != CROSS_PAGE_PROOF {
        return Err(verification_error(
            "cross-page usercopy proof",
            format!("expected {CROSS_PAGE_PROOF:?}, got {proof:?}"),
        ));
    }
    for (io_exit, expected) in execution
        .io_exits()
        .iter()
        .zip(CROSS_PAGE_PROOF.iter().copied())
    {
        if io_exit.direction() != PortIoDirection::Out
            || io_exit.size() != 1
            || io_exit.count() != 1
            || io_exit.output_data() != [expected]
        {
            return Err(verification_error(
                "cross-page usercopy port I/O metadata",
                format!("unexpected exit {io_exit:?} for byte {expected:#x}"),
            ));
        }
    }

    let register_snapshot = vcpu.capture_register_snapshot()?;
    let terminal_regs = vcpu.registers()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered cross-page usercopy memory remains VM-owned");
    let returns = read_returns(guest_memory)?;
    let good_source = read_four(guest_memory, CROSS_PAGE_GOOD_SOURCE)?;
    let good_destination = read_four(guest_memory, CROSS_PAGE_GOOD_DESTINATION)?;
    let source_fault_source = read_four(guest_memory, CROSS_PAGE_SOURCE_FAULT_SOURCE)?;
    let source_fault_destination = read_four(guest_memory, CROSS_PAGE_SOURCE_FAULT_DESTINATION)?;
    let destination_fault_source = read_four(guest_memory, CROSS_PAGE_DEST_FAULT_SOURCE)?;
    let destination_fault_destination = read_four(guest_memory, CROSS_PAGE_DEST_FAULT_DESTINATION)?;
    let read_fault = read_fault_observation(guest_memory, CROSS_PAGE_READ_FAULT_OBSERVATION_ADDR)?;
    let write_fault =
        read_fault_observation(guest_memory, CROSS_PAGE_WRITE_FAULT_OBSERVATION_ADDR)?;
    let fixup_entries = read_fixup_table(guest_memory)?;
    let terminal_frame = read_terminal_frame(guest_memory)?;
    let user_page_ptes = USER_PAGES
        .iter()
        .map(|(page, _)| Ok((*page, read_pte(guest_memory, *page)?)))
        .collect::<Result<Vec<_>, Error>>()?;
    let service_pte = read_pte(guest_memory, SYSCALL_KERNEL_ENTRY.get())?;
    let fault_handler_pte = read_pte(guest_memory, CROSS_PAGE_PAGE_FAULT_HANDLER.get())?;
    let fault_metadata_pte = read_pte(guest_memory, CROSS_PAGE_READ_FAULT_OBSERVATION_ADDR.get())?;

    validate_runtime_state(
        execution.report(),
        terminal_return_rip,
        returns,
        good_source,
        good_destination,
        source_fault_source,
        source_fault_destination,
        destination_fault_source,
        destination_fault_destination,
        read_fault,
        write_fault,
        fixup_entries,
        &user_page_ptes,
        service_pte,
        fault_handler_pte,
        fault_metadata_pte,
        terminal_frame,
        register_snapshot.rsp(),
        special.cs().selector(),
        terminal_regs.rflags,
        special.cr2(),
        msrs,
    )?;

    Ok(CrossPageUsercopyGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
        returns,
        good_source,
        good_destination,
        source_fault_source,
        source_fault_destination,
        destination_fault_source,
        destination_fault_destination,
        read_fault,
        write_fault,
        fixup_entries,
        terminal_frame,
        terminal_rsp: register_snapshot.rsp(),
        terminal_cs: special.cs().selector(),
        terminal_rflags: terminal_regs.rflags,
        final_cr2: special.cr2(),
        msrs,
        user_page_ptes,
        service_pte,
        fault_handler_pte,
        fault_metadata_pte,
    })
}

fn build_user_guest() -> Vec<u8> {
    let mut code = Vec::new();
    emit_movabs(&mut code, 0xbb, CROSS_PAGE_RESULT_ADDR.get());
    emit_copy_call(
        &mut code,
        CROSS_PAGE_GOOD_SOURCE,
        CROSS_PAGE_GOOD_DESTINATION,
        0,
    );
    emit_copy_call(
        &mut code,
        CROSS_PAGE_SOURCE_FAULT_SOURCE,
        CROSS_PAGE_SOURCE_FAULT_DESTINATION,
        8,
    );
    emit_copy_call(
        &mut code,
        CROSS_PAGE_DEST_FAULT_SOURCE,
        CROSS_PAGE_DEST_FAULT_DESTINATION,
        16,
    );
    code.extend_from_slice(&[0xcd, 0x81]);
    code
}

fn emit_copy_call(code: &mut Vec<u8>, source: u64, destination: u64, result_offset: u8) {
    emit_movabs(code, 0xbf, source);
    emit_movabs(code, 0xbe, destination);
    code.extend_from_slice(&[0x0f, 0x05]);
    if result_offset == 0 {
        code.extend_from_slice(&[0x48, 0x89, 0x03]);
    } else {
        code.extend_from_slice(&[0x48, 0x89, 0x43, result_offset]);
    }
    code.extend_from_slice(&[0xcd, 0x80]);
}

fn emit_movabs(code: &mut Vec<u8>, opcode: u8, value: u64) {
    code.extend_from_slice(&[0x48, opcode]);
    code.extend_from_slice(&value.to_le_bytes());
}

fn install_cross_page_mappings(memory: &mut GuestMemory) -> Result<(), Error> {
    for (page, present) in USER_PAGES {
        let flags = X86_PAGE_WRITE | X86_PAGE_USER | if present { X86_PAGE_PRESENT } else { 0 };
        write_u64(
            memory,
            GuestPhysAddr::new(PRIVILEGE_PT_ADDR.get() + page / LONG_MODE_PAGE_SIZE * 8),
            page | flags,
        )?;
    }
    Ok(())
}

fn install_page_fault_gate(memory: &mut GuestMemory) -> Result<(), Error> {
    memory.write(
        GuestPhysAddr::new(
            PRIVILEGE_IDT_ADDR.get()
                + u64::from(CROSS_PAGE_PAGE_FAULT_VECTOR) * PAGE_FAULT_GATE_SIZE,
        ),
        &encode_kernel_interrupt_gate(CROSS_PAGE_PAGE_FAULT_HANDLER.get()),
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

fn initialize_data(memory: &mut GuestMemory) -> Result<(), Error> {
    memory.write(
        GuestPhysAddr::new(CROSS_PAGE_GOOD_SOURCE),
        &CROSS_PAGE_GOOD_BYTES,
    )?;
    memory.write(GuestPhysAddr::new(CROSS_PAGE_GOOD_DESTINATION), &[0; 4])?;
    memory.write(
        GuestPhysAddr::new(CROSS_PAGE_SOURCE_FAULT_SOURCE),
        &CROSS_PAGE_SOURCE_FAULT_BYTES,
    )?;
    memory.write(
        GuestPhysAddr::new(CROSS_PAGE_SOURCE_FAULT_DESTINATION),
        &[0; 4],
    )?;
    memory.write(
        GuestPhysAddr::new(CROSS_PAGE_DEST_FAULT_SOURCE),
        &CROSS_PAGE_DEST_FAULT_BYTES,
    )?;
    memory.write(
        GuestPhysAddr::new(CROSS_PAGE_DEST_FAULT_DESTINATION),
        &[0; 4],
    )?;
    Ok(())
}

fn expected_fixup_entries() -> [CrossPageFixupEntry; 2] {
    [
        CrossPageFixupEntry {
            fault_rip: CROSS_PAGE_READ_FAULT_RIP,
            fixup_rip: CROSS_PAGE_COMMON_FIXUP_RIP,
            observation_addr: CROSS_PAGE_READ_FAULT_OBSERVATION_ADDR.get(),
        },
        CrossPageFixupEntry {
            fault_rip: CROSS_PAGE_WRITE_FAULT_RIP,
            fixup_rip: CROSS_PAGE_COMMON_FIXUP_RIP,
            observation_addr: CROSS_PAGE_WRITE_FAULT_OBSERVATION_ADDR.get(),
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

fn read_returns(memory: &GuestMemory) -> Result<[u64; 3], Error> {
    let mut bytes = [0_u8; 24];
    memory.read(CROSS_PAGE_RESULT_ADDR, &mut bytes)?;
    Ok([
        read_u64(&bytes, 0),
        read_u64(&bytes, 8),
        read_u64(&bytes, 16),
    ])
}

fn read_four(memory: &GuestMemory, address: u64) -> Result<[u8; 4], Error> {
    let mut bytes = [0_u8; 4];
    memory.read(GuestPhysAddr::new(address), &mut bytes)?;
    Ok(bytes)
}

fn read_fault_observation(
    memory: &GuestMemory,
    address: GuestPhysAddr,
) -> Result<CrossPageFaultObservation, Error> {
    let mut bytes = [0_u8; FAULT_OBSERVATION_BYTES];
    memory.read(address, &mut bytes)?;
    Ok(CrossPageFaultObservation {
        cr2: read_u64(&bytes, 0),
        error_code: read_u64(&bytes, 8),
        rip: read_u64(&bytes, 16),
        cs: read_u64(&bytes, 24),
        rflags: read_u64(&bytes, 32),
        resolved_fixup: read_u64(&bytes, 40),
    })
}

fn read_fixup_table(memory: &GuestMemory) -> Result<[CrossPageFixupEntry; 2], Error> {
    let mut bytes = [0_u8; FIXUP_TABLE_BYTES];
    memory.read(CROSS_PAGE_FIXUP_TABLE_ADDR, &mut bytes)?;
    Ok([
        CrossPageFixupEntry {
            fault_rip: read_u64(&bytes, 0),
            fixup_rip: read_u64(&bytes, 8),
            observation_addr: read_u64(&bytes, 16),
        },
        CrossPageFixupEntry {
            fault_rip: read_u64(&bytes, 24),
            fixup_rip: read_u64(&bytes, 32),
            observation_addr: read_u64(&bytes, 40),
        },
    ])
}

fn read_terminal_frame(memory: &GuestMemory) -> Result<CrossPageTerminalFrame, Error> {
    let start = GuestPhysAddr::new(PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES);
    let mut bytes = [0_u8; PRIVILEGE_FRAME_BYTES as usize];
    memory.read(start, &mut bytes)?;
    Ok(CrossPageTerminalFrame {
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
            .expect("fixed cross-page field is eight bytes"),
    )
}

fn write_u64(memory: &mut GuestMemory, address: GuestPhysAddr, value: u64) -> Result<(), Error> {
    memory.write(address, &value.to_le_bytes())
}

#[allow(clippy::too_many_arguments)]
fn validate_runtime_state(
    report: VmExitReport,
    terminal_return_rip: u64,
    returns: [u64; 3],
    good_source: [u8; 4],
    good_destination: [u8; 4],
    source_fault_source: [u8; 4],
    source_fault_destination: [u8; 4],
    destination_fault_source: [u8; 4],
    destination_fault_destination: [u8; 4],
    read_fault: CrossPageFaultObservation,
    write_fault: CrossPageFaultObservation,
    fixup_entries: [CrossPageFixupEntry; 2],
    user_page_ptes: &[(u64, u64)],
    service_pte: u64,
    fault_handler_pte: u64,
    fault_metadata_pte: u64,
    terminal_frame: CrossPageTerminalFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    final_cr2: u64,
    msrs: [u64; 4],
) -> Result<(), Error> {
    let expected_read_fault = CrossPageFaultObservation {
        cr2: CROSS_PAGE_SOURCE_FAULT_ADDR,
        error_code: 0,
        rip: CROSS_PAGE_READ_FAULT_RIP,
        cs: u64::from(KERNEL_CODE_SELECTOR),
        rflags: PAGE_FAULT_SAVED_RFLAGS,
        resolved_fixup: CROSS_PAGE_COMMON_FIXUP_RIP,
    };
    let expected_write_fault = CrossPageFaultObservation {
        cr2: CROSS_PAGE_DEST_FAULT_ADDR,
        error_code: 1 << 1,
        rip: CROSS_PAGE_WRITE_FAULT_RIP,
        cs: u64::from(KERNEL_CODE_SELECTOR),
        rflags: PAGE_FAULT_SAVED_RFLAGS,
        resolved_fixup: CROSS_PAGE_COMMON_FIXUP_RIP,
    };
    let expected_frame = CrossPageTerminalFrame {
        rip: terminal_return_rip,
        cs: u64::from(PRIVILEGE_USER_CODE_SELECTOR),
        rflags: X86_RFLAGS_RESERVED | X86_RFLAGS_IF,
        rsp: PRIVILEGE_USER_STACK,
        ss: u64::from(PRIVILEGE_USER_DATA_SELECTOR),
    };
    let expected_source_fault_destination = [
        CROSS_PAGE_SOURCE_FAULT_BYTES[0],
        CROSS_PAGE_SOURCE_FAULT_BYTES[1],
        0,
        0,
    ];
    let expected_destination_fault_destination = [
        CROSS_PAGE_DEST_FAULT_BYTES[0],
        CROSS_PAGE_DEST_FAULT_BYTES[1],
        0,
        0,
    ];

    let mappings_valid = user_page_ptes.len() == USER_PAGES.len()
        && user_page_ptes.iter().zip(USER_PAGES).all(
            |((address, pte), (expected_address, present))| {
                *address == expected_address
                    && pte & (X86_PAGE_WRITE | X86_PAGE_USER) == (X86_PAGE_WRITE | X86_PAGE_USER)
                    && (pte & X86_PAGE_PRESENT != 0) == present
            },
        );

    if returns != [CROSS_PAGE_COPY_LEN, 2, 2]
        || good_source != CROSS_PAGE_GOOD_BYTES
        || good_destination != CROSS_PAGE_GOOD_BYTES
        || source_fault_source != CROSS_PAGE_SOURCE_FAULT_BYTES
        || source_fault_destination != expected_source_fault_destination
        || destination_fault_source != CROSS_PAGE_DEST_FAULT_BYTES
        || destination_fault_destination != expected_destination_fault_destination
        || read_fault != expected_read_fault
        || write_fault != expected_write_fault
        || fixup_entries != expected_fixup_entries()
        || !mappings_valid
        || service_pte & X86_PAGE_USER != 0
        || fault_handler_pte & X86_PAGE_USER != 0
        || fault_metadata_pte & X86_PAGE_USER != 0
        || terminal_frame != expected_frame
        || terminal_rsp != PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES
        || terminal_cs != KERNEL_CODE_SELECTOR
        || terminal_rflags & X86_RFLAGS_RESERVED != X86_RFLAGS_RESERVED
        || terminal_rflags & X86_RFLAGS_IF != 0
        || final_cr2 != CROSS_PAGE_DEST_FAULT_ADDR
        || msrs[0] & EFER_SYSCALL_ENABLE != EFER_SYSCALL_ENABLE
        || msrs[1] != SYSCALL_STAR_VALUE
        || msrs[2] != SYSCALL_LSTAR_VALUE
        || msrs[3] != SYSCALL_SFMASK_VALUE
        || report.exit() != VcpuExit::Hlt
        || report.rip() != PRIVILEGE_TERMINAL_HANDLER.get() + 5
    {
        return Err(verification_error(
            "cross-page usercopy architectural state",
            format!(
                "returns={returns:?} good={good_source:?}->{good_destination:?} source_fault={source_fault_source:?}->{source_fault_destination:?} destination_fault={destination_fault_source:?}->{destination_fault_destination:?} read_fault={read_fault:?} write_fault={write_fault:?} fixups={fixup_entries:?} terminal={terminal_frame:?}/{terminal_rsp:#x}/{terminal_cs:#x}/{terminal_rflags:#x} final_cr2={final_cr2:#x} report={report}"
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

    #[test]
    fn service_has_one_load_site_one_store_site_and_progress_preserving_fixup() {
        assert_eq!(COPY_SERVICE_BYTES.len(), 46);
        assert_eq!(COPY_SERVICE_BYTES[19..24], [0x42, 0x0f, 0xb6, 0x04, 0x07]);
        assert_eq!(COPY_SERVICE_BYTES[24..28], [0x42, 0x88, 0x04, 0x06]);
        assert_eq!(COPY_SERVICE_BYTES[35..37], [0x75, 0xeb]);
        assert_eq!(CROSS_PAGE_READ_FAULT_RIP, SYSCALL_KERNEL_ENTRY.get() + 19);
        assert_eq!(CROSS_PAGE_WRITE_FAULT_RIP, SYSCALL_KERNEL_ENTRY.get() + 24);
        assert_eq!(CROSS_PAGE_COMMON_FIXUP_RIP, SYSCALL_KERNEL_ENTRY.get() + 37);
        assert_eq!(PAGE_FAULT_HANDLER_BYTES.len(), 105);
        assert!(!PAGE_FAULT_HANDLER_BYTES
            .windows(3)
            .any(|bytes| bytes == [0x49, 0xc7, 0xc0]));
    }

    #[test]
    fn fixture_places_faults_on_third_byte_of_cross_page_copy() {
        assert_eq!(CROSS_PAGE_GOOD_SOURCE & 0xfff, 0xffe);
        assert_eq!(CROSS_PAGE_GOOD_DESTINATION & 0xfff, 0xffe);
        assert_eq!(
            CROSS_PAGE_SOURCE_FAULT_SOURCE + 2,
            CROSS_PAGE_SOURCE_FAULT_ADDR
        );
        assert_eq!(
            CROSS_PAGE_DEST_FAULT_DESTINATION + 2,
            CROSS_PAGE_DEST_FAULT_ADDR
        );
        assert_eq!(
            USER_PAGES.iter().find(|(page, _)| *page == 0x25000),
            Some(&(0x25000, false))
        );
        assert_eq!(
            USER_PAGES.iter().find(|(page, _)| *page == 0x2b000),
            Some(&(0x2b000, false))
        );
    }

    #[test]
    fn user_program_issues_three_syscalls_and_three_ring3_return_markers() {
        let code = build_user_guest();
        assert_eq!(
            code.windows(2)
                .filter(|bytes| *bytes == [0x0f, 0x05])
                .count(),
            3
        );
        assert_eq!(
            code.windows(2)
                .filter(|bytes| *bytes == [0xcd, 0x80])
                .count(),
            3
        );
        assert_eq!(&code[code.len() - 2..], [0xcd, 0x81]);
    }
}
