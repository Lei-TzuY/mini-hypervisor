use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE};
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

pub const PARTIAL_COPY_NR: u64 = 2;
pub const PARTIAL_COPY_MAX_LEN: u64 = 4;
pub const PARTIAL_GOOD_SOURCE: u64 = 0x20ffe;
pub const PARTIAL_GOOD_DESTINATION: u64 = 0x22ffe;
pub const PARTIAL_SOURCE_FAULT_SOURCE: u64 = 0x24ffe;
pub const PARTIAL_SOURCE_FAULT_DESTINATION: u64 = 0x26ffe;
pub const PARTIAL_DEST_FAULT_SOURCE: u64 = 0x28ffe;
pub const PARTIAL_DEST_FAULT_DESTINATION: u64 = 0x2affe;
pub const PARTIAL_SOURCE_FAULT_ADDR: u64 = 0x25000;
pub const PARTIAL_DEST_FAULT_ADDR: u64 = 0x2b000;
pub const PARTIAL_SHORT_SOURCE: u64 = 0x20020;
pub const PARTIAL_SHORT_DESTINATION: u64 = 0x22020;
pub const PARTIAL_BYTE_SOURCE: u64 = 0x20040;
pub const PARTIAL_BYTE_DESTINATION: u64 = 0x22040;
pub const PARTIAL_RESULT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xa180);
pub const PARTIAL_READ_FAULT_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb000);
pub const PARTIAL_WRITE_FAULT_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb040);
pub const PARTIAL_FIXUP_TABLE_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xb100);
pub const PARTIAL_PAGE_FAULT_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_4000);
pub const PARTIAL_PAGE_FAULT_VECTOR: u8 = 14;
pub const PARTIAL_READ_FAULT_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 0x78;
pub const PARTIAL_WRITE_FAULT_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 0x7d;
pub const PARTIAL_COMMON_FIXUP_RIP: u64 = SYSCALL_KERNEL_ENTRY.get() + 0x98;
pub const PARTIAL_EINVAL: u64 = (-22_i64) as u64;
pub const PARTIAL_ENOSYS: u64 = (-38_i64) as u64;
pub const PARTIAL_PROOF: &[u8; 10] = b"MFFMIICPUD";

pub const PARTIAL_GOOD_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
pub const PARTIAL_SOURCE_FAULT_BYTES: [u8; 4] = [0x55, 0x66, 0x77, 0x88];
pub const PARTIAL_DEST_FAULT_BYTES: [u8; 4] = [0x99, 0xaa, 0xbb, 0xcc];
pub const PARTIAL_SHORT_BYTES: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];
pub const PARTIAL_BYTE_VALUE: u8 = 0x6b;

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_RFLAGS_RESERVED: u64 = 1 << 1;
const X86_RFLAGS_PF: u64 = 1 << 2;
const X86_RFLAGS_ZF: u64 = 1 << 6;
const X86_RFLAGS_IF: u64 = 1 << 9;
const X86_RFLAGS_RF: u64 = 1 << 16;
const PARTIAL_EXIT_BUDGET: u32 = 11;
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

// The first 90 bytes retain the integrated nr0/nr1 dispatcher layout. Bytes 25..38 are a
// fixed-size nr2/unknown trampoline, so the original byte-copy fault sites and common return stay at
// LSTAR+38, +41, +51, +64 and +84 respectively. The bounded range service is append-only.
const PARTIAL_DISPATCHER_BYTES: [u8; 180] = [
    0x49, 0x89, 0xe2, 0x48, 0xbc, 0x00, 0xe0, 0x1f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x83, 0xf8,
    0x00, 0x74, 0x13, 0x48, 0x83, 0xf8, 0x01, 0x74, 0x34, 0x48, 0x83, 0xf8, 0x02, 0x74, 0x48, 0xeb,
    0x39, 0x90, 0x90, 0x90, 0x90, 0x90, 0x0f, 0xb6, 0x07, 0x88, 0x06, 0xb0, b'C', 0xe6, 0xe9, 0x31,
    0xc0, 0xeb, 0x21, 0xb0, b'R', 0xe6, 0xe9, 0x48, 0xc7, 0xc0, 0xf2, 0xff, 0xff, 0xff, 0xeb, 0x14,
    0xb0, b'W', 0xe6, 0xe9, 0x48, 0xc7, 0xc0, 0xf2, 0xff, 0xff, 0xff, 0xeb, 0x07, 0x40, 0x88, 0xf8,
    0xe6, 0xe9, 0x31, 0xc0, 0x4c, 0x89, 0xd4, 0x48, 0x0f, 0x07, 0xb0, b'U', 0xe6, 0xe9, 0x48, 0xc7,
    0xc0, 0xda, 0xff, 0xff, 0xff, 0xeb, 0xed, 0x48, 0x85, 0xd2, 0x74, 0x3b, 0x48, 0x83, 0xfa, 0x04,
    0x77, 0x35, 0x45, 0x31, 0xc0, 0x4d, 0x39, 0xc0, 0x42, 0x0f, 0xb6, 0x04, 0x07, 0x42, 0x88, 0x04,
    0x06, 0x49, 0xff, 0xc0, 0x49, 0x39, 0xd0, 0x72, 0xec, 0x4c, 0x89, 0xc0, 0x49, 0x89, 0xc1, 0xb0,
    b'M', 0xe6, 0xe9, 0x4c, 0x89, 0xc8, 0xeb, 0xbc, 0x4c, 0x89, 0xc0, 0x49, 0x89, 0xc1, 0xb0, b'F',
    0xe6, 0xe9, 0x4c, 0x89, 0xc8, 0xeb, 0xad, 0xb0, b'I', 0xe6, 0xe9, 0x48, 0xc7, 0xc0, 0xea, 0xff,
    0xff, 0xff, 0xeb, 0xa0,
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
pub struct PartialDispatchFixupEntry {
    fault_rip: u64,
    fixup_rip: u64,
    observation_addr: u64,
}

impl PartialDispatchFixupEntry {
    #[must_use]
    pub const fn fault_rip(self) -> u64 { self.fault_rip }
    #[must_use]
    pub const fn fixup_rip(self) -> u64 { self.fixup_rip }
    #[must_use]
    pub const fn observation_addr(self) -> u64 { self.observation_addr }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialDispatchFaultObservation {
    cr2: u64,
    error_code: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    resolved_fixup: u64,
}

impl PartialDispatchFaultObservation {
    #[must_use]
    pub const fn cr2(self) -> u64 { self.cr2 }
    #[must_use]
    pub const fn error_code(self) -> u64 { self.error_code }
    #[must_use]
    pub const fn rip(self) -> u64 { self.rip }
    #[must_use]
    pub const fn cs(self) -> u64 { self.cs }
    #[must_use]
    pub const fn rflags(self) -> u64 { self.rflags }
    #[must_use]
    pub const fn resolved_fixup(self) -> u64 { self.resolved_fixup }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialDispatchTerminalFrame {
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

impl PartialDispatchTerminalFrame {
    #[must_use]
    pub const fn rip(self) -> u64 { self.rip }
    #[must_use]
    pub const fn cs(self) -> u64 { self.cs }
    #[must_use]
    pub const fn rflags(self) -> u64 { self.rflags }
    #[must_use]
    pub const fn rsp(self) -> u64 { self.rsp }
    #[must_use]
    pub const fn ss(self) -> u64 { self.ss }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialDispatchGuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
    returns: [u64; 9],
    good_destination: [u8; 4],
    source_fault_destination: [u8; 4],
    destination_fault_destination: [u8; 4],
    short_destination: [u8; 4],
    byte_destination: u8,
    read_fault: PartialDispatchFaultObservation,
    write_fault: PartialDispatchFaultObservation,
    fixup_entries: [PartialDispatchFixupEntry; 2],
    terminal_frame: PartialDispatchTerminalFrame,
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

impl PartialDispatchGuestResult {
    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] { &self.io_exits }
    #[must_use]
    pub fn proof(&self) -> &[u8] { &self.proof }
    #[must_use]
    pub const fn report(&self) -> VmExitReport { self.report }
    #[must_use]
    pub const fn returns(&self) -> [u64; 9] { self.returns }
    #[must_use]
    pub const fn good_destination(&self) -> [u8; 4] { self.good_destination }
    #[must_use]
    pub const fn source_fault_destination(&self) -> [u8; 4] { self.source_fault_destination }
    #[must_use]
    pub const fn destination_fault_destination(&self) -> [u8; 4] { self.destination_fault_destination }
    #[must_use]
    pub const fn short_destination(&self) -> [u8; 4] { self.short_destination }
    #[must_use]
    pub const fn byte_destination(&self) -> u8 { self.byte_destination }
    #[must_use]
    pub const fn read_fault(&self) -> PartialDispatchFaultObservation { self.read_fault }
    #[must_use]
    pub const fn write_fault(&self) -> PartialDispatchFaultObservation { self.write_fault }
    #[must_use]
    pub const fn fixup_entries(&self) -> &[PartialDispatchFixupEntry; 2] { &self.fixup_entries }
    #[must_use]
    pub const fn terminal_frame(&self) -> PartialDispatchTerminalFrame { self.terminal_frame }
    #[must_use]
    pub const fn terminal_rsp(&self) -> u64 { self.terminal_rsp }
    #[must_use]
    pub const fn terminal_cs(&self) -> u16 { self.terminal_cs }
    #[must_use]
    pub const fn terminal_rflags(&self) -> u64 { self.terminal_rflags }
    #[must_use]
    pub const fn final_cr2(&self) -> u64 { self.final_cr2 }
    #[must_use]
    pub const fn msrs(&self) -> [u64; 4] { self.msrs }
    #[must_use]
    pub fn user_page_ptes(&self) -> &[(u64, u64)] { &self.user_page_ptes }
    #[must_use]
    pub const fn service_pte(&self) -> u64 { self.service_pte }
    #[must_use]
    pub const fn fault_handler_pte(&self) -> u64 { self.fault_handler_pte }
    #[must_use]
    pub const fn fault_metadata_pte(&self) -> u64 { self.fault_metadata_pte }
}

pub fn run_partial_dispatch_guest(config: VmConfig) -> Result<PartialDispatchGuestResult, Error> {
    let user_bytes = build_user_guest();
    let terminal_return_rip = PRIVILEGE_USER_ENTRY.get()
        + u64::try_from(user_bytes.len()).expect("bounded partial-dispatch user program fits u64");
    let kernel = FlatGuestImage::new(PRIVILEGE_KERNEL_ENTRY, PRIVILEGE_KERNEL_ENTRY, &KERNEL_BOOT_BYTES)?;
    let user = FlatGuestImage::new(PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_ENTRY, &user_bytes)?;
    let dispatcher = FlatGuestImage::new(SYSCALL_KERNEL_ENTRY, SYSCALL_KERNEL_ENTRY, &PARTIAL_DISPATCHER_BYTES)?;
    let page_fault_handler = FlatGuestImage::new(
        PARTIAL_PAGE_FAULT_HANDLER,
        PARTIAL_PAGE_FAULT_HANDLER,
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
        .expect("bounded partial-copy dispatcher privilege layout remains valid");
    layout.install_tables(&mut memory)?;
    install_user_mappings(&mut memory)?;
    install_page_fault_gate(&mut memory)?;
    kernel.load(&mut memory)?;
    user.load(&mut memory)?;
    dispatcher.load(&mut memory)?;
    page_fault_handler.load(&mut memory)?;
    terminal_handler.load(&mut memory)?;
    initialize_data(&mut memory)?;
    memory.write(PARTIAL_RESULT_ADDR, &[0; 72])?;
    memory.write(PARTIAL_READ_FAULT_OBSERVATION_ADDR, &[0; FAULT_OBSERVATION_BYTES])?;
    memory.write(PARTIAL_WRITE_FAULT_OBSERVATION_ADDR, &[0; FAULT_OBSERVATION_BYTES])?;
    memory.write(PARTIAL_FIXUP_TABLE_ADDR, &encoded_fixup_table())?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(&layout)?;
    let msrs = configure_syscall_msrs(&backend, &vcpu)?;

    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, PARTIAL_EXIT_BUDGET)?;
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if execution.io_exits().len() != PARTIAL_PROOF.len() || proof.as_slice() != PARTIAL_PROOF {
        return Err(verification_error(
            "partial-copy dispatcher proof",
            format!("expected {PARTIAL_PROOF:?}, got {proof:?} report={}", execution.report()),
        ));
    }
    for (io, expected) in execution.io_exits().iter().zip(PARTIAL_PROOF.iter().copied()) {
        if io.direction() != PortIoDirection::Out
            || io.size() != 1
            || io.count() != 1
            || io.output_data() != [expected]
        {
            return Err(verification_error(
                "partial-copy dispatcher port I/O metadata",
                format!("unexpected exit {io:?} for byte {expected:#x}"),
            ));
        }
    }

    let register_snapshot = vcpu.capture_register_snapshot()?;
    let terminal_regs = vcpu.registers()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered partial-dispatch memory remains VM-owned");
    let result = PartialDispatchGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
        returns: read_returns(guest_memory)?,
        good_destination: read_four(guest_memory, PARTIAL_GOOD_DESTINATION)?,
        source_fault_destination: read_four(guest_memory, PARTIAL_SOURCE_FAULT_DESTINATION)?,
        destination_fault_destination: read_four(guest_memory, PARTIAL_DEST_FAULT_DESTINATION)?,
        short_destination: read_four(guest_memory, PARTIAL_SHORT_DESTINATION)?,
        byte_destination: read_byte(guest_memory, PARTIAL_BYTE_DESTINATION)?,
        read_fault: read_fault_observation(guest_memory, PARTIAL_READ_FAULT_OBSERVATION_ADDR)?,
        write_fault: read_fault_observation(guest_memory, PARTIAL_WRITE_FAULT_OBSERVATION_ADDR)?,
        fixup_entries: read_fixup_table(guest_memory)?,
        terminal_frame: read_terminal_frame(guest_memory)?,
        terminal_rsp: register_snapshot.rsp(),
        terminal_cs: special.cs().selector(),
        terminal_rflags: terminal_regs.rflags,
        final_cr2: special.cr2(),
        msrs,
        user_page_ptes: USER_PAGES
            .iter()
            .map(|(page, _)| Ok((*page, read_pte(guest_memory, *page)?)))
            .collect::<Result<Vec<_>, Error>>()?,
        service_pte: read_pte(guest_memory, SYSCALL_KERNEL_ENTRY.get())?,
        fault_handler_pte: read_pte(guest_memory, PARTIAL_PAGE_FAULT_HANDLER.get())?,
        fault_metadata_pte: read_pte(guest_memory, PARTIAL_READ_FAULT_OBSERVATION_ADDR.get())?,
    };
    validate_runtime_state(&result, terminal_return_rip)?;
    Ok(result)
}

fn build_user_guest() -> Vec<u8> {
    let mut code = Vec::new();
    emit_movabs(&mut code, 0xbb, PARTIAL_RESULT_ADDR.get());
    emit_range_call(&mut code, PARTIAL_GOOD_SOURCE, PARTIAL_GOOD_DESTINATION, 4, 0);
    emit_range_call(
        &mut code,
        PARTIAL_SOURCE_FAULT_SOURCE,
        PARTIAL_SOURCE_FAULT_DESTINATION,
        4,
        8,
    );
    emit_range_call(
        &mut code,
        PARTIAL_DEST_FAULT_SOURCE,
        PARTIAL_DEST_FAULT_DESTINATION,
        4,
        16,
    );
    emit_range_call(&mut code, PARTIAL_SHORT_SOURCE, PARTIAL_SHORT_DESTINATION, 1, 24);
    emit_range_call(&mut code, PARTIAL_SHORT_SOURCE, PARTIAL_SHORT_DESTINATION, 0, 32);
    emit_range_call(&mut code, PARTIAL_SHORT_SOURCE, PARTIAL_SHORT_DESTINATION, 5, 40);
    emit_copy_byte_call(&mut code, 48);
    emit_putc_call(&mut code, 56);
    emit_unknown_call(&mut code, 64);
    code.extend_from_slice(&[0xcd, 0x81]);
    code
}

fn emit_range_call(code: &mut Vec<u8>, source: u64, destination: u64, len: u32, result_offset: u8) {
    code.extend_from_slice(&[0xb8, PARTIAL_COPY_NR as u8, 0, 0, 0]);
    emit_movabs(code, 0xbf, source);
    emit_movabs(code, 0xbe, destination);
    code.extend_from_slice(&[0xba]);
    code.extend_from_slice(&len.to_le_bytes());
    code.extend_from_slice(&[0x0f, 0x05]);
    emit_store_result(code, result_offset);
}

fn emit_copy_byte_call(code: &mut Vec<u8>, result_offset: u8) {
    code.extend_from_slice(&[0xb8, 0, 0, 0, 0]);
    emit_movabs(code, 0xbf, PARTIAL_BYTE_SOURCE);
    emit_movabs(code, 0xbe, PARTIAL_BYTE_DESTINATION);
    code.extend_from_slice(&[0x0f, 0x05]);
    emit_store_result(code, result_offset);
}

fn emit_putc_call(code: &mut Vec<u8>, result_offset: u8) {
    code.extend_from_slice(&[0xb8, 1, 0, 0, 0]);
    code.extend_from_slice(&[0xbf, b'P', 0, 0, 0]);
    code.extend_from_slice(&[0x0f, 0x05]);
    emit_store_result(code, result_offset);
}

fn emit_unknown_call(code: &mut Vec<u8>, result_offset: u8) {
    code.extend_from_slice(&[0xb8, 0xff, 0, 0, 0, 0x0f, 0x05]);
    emit_store_result(code, result_offset);
}

fn emit_movabs(code: &mut Vec<u8>, opcode: u8, value: u64) {
    code.extend_from_slice(&[0x48, opcode]);
    code.extend_from_slice(&value.to_le_bytes());
}

fn emit_store_result(code: &mut Vec<u8>, result_offset: u8) {
    if result_offset == 0 {
        code.extend_from_slice(&[0x48, 0x89, 0x03]);
    } else {
        code.extend_from_slice(&[0x48, 0x89, 0x43, result_offset]);
    }
}

fn install_user_mappings(memory: &mut GuestMemory) -> Result<(), Error> {
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
                + u64::from(PARTIAL_PAGE_FAULT_VECTOR) * PAGE_FAULT_GATE_SIZE,
        ),
        &encode_kernel_interrupt_gate(PARTIAL_PAGE_FAULT_HANDLER.get()),
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
    memory.write(GuestPhysAddr::new(PARTIAL_GOOD_SOURCE), &PARTIAL_GOOD_BYTES)?;
    memory.write(GuestPhysAddr::new(PARTIAL_GOOD_DESTINATION), &[0; 4])?;
    memory.write(
        GuestPhysAddr::new(PARTIAL_SOURCE_FAULT_SOURCE),
        &PARTIAL_SOURCE_FAULT_BYTES,
    )?;
    memory.write(GuestPhysAddr::new(PARTIAL_SOURCE_FAULT_DESTINATION), &[0; 4])?;
    memory.write(
        GuestPhysAddr::new(PARTIAL_DEST_FAULT_SOURCE),
        &PARTIAL_DEST_FAULT_BYTES,
    )?;
    memory.write(GuestPhysAddr::new(PARTIAL_DEST_FAULT_DESTINATION), &[0; 4])?;
    memory.write(GuestPhysAddr::new(PARTIAL_SHORT_SOURCE), &PARTIAL_SHORT_BYTES)?;
    memory.write(GuestPhysAddr::new(PARTIAL_SHORT_DESTINATION), &[0; 4])?;
    memory.write(GuestPhysAddr::new(PARTIAL_BYTE_SOURCE), &[PARTIAL_BYTE_VALUE])?;
    memory.write(GuestPhysAddr::new(PARTIAL_BYTE_DESTINATION), &[0])?;
    Ok(())
}

fn expected_fixup_entries() -> [PartialDispatchFixupEntry; 2] {
    [
        PartialDispatchFixupEntry {
            fault_rip: PARTIAL_READ_FAULT_RIP,
            fixup_rip: PARTIAL_COMMON_FIXUP_RIP,
            observation_addr: PARTIAL_READ_FAULT_OBSERVATION_ADDR.get(),
        },
        PartialDispatchFixupEntry {
            fault_rip: PARTIAL_WRITE_FAULT_RIP,
            fixup_rip: PARTIAL_COMMON_FIXUP_RIP,
            observation_addr: PARTIAL_WRITE_FAULT_OBSERVATION_ADDR.get(),
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

fn read_returns(memory: &GuestMemory) -> Result<[u64; 9], Error> {
    let mut bytes = [0_u8; 72];
    memory.read(PARTIAL_RESULT_ADDR, &mut bytes)?;
    Ok(std::array::from_fn(|index| read_u64(&bytes, index * 8)))
}

fn read_four(memory: &GuestMemory, address: u64) -> Result<[u8; 4], Error> {
    let mut bytes = [0_u8; 4];
    memory.read(GuestPhysAddr::new(address), &mut bytes)?;
    Ok(bytes)
}

fn read_byte(memory: &GuestMemory, address: u64) -> Result<u8, Error> {
    let mut byte = [0_u8; 1];
    memory.read(GuestPhysAddr::new(address), &mut byte)?;
    Ok(byte[0])
}

fn read_fault_observation(
    memory: &GuestMemory,
    address: GuestPhysAddr,
) -> Result<PartialDispatchFaultObservation, Error> {
    let mut bytes = [0_u8; FAULT_OBSERVATION_BYTES];
    memory.read(address, &mut bytes)?;
    Ok(PartialDispatchFaultObservation {
        cr2: read_u64(&bytes, 0),
        error_code: read_u64(&bytes, 8),
        rip: read_u64(&bytes, 16),
        cs: read_u64(&bytes, 24),
        rflags: read_u64(&bytes, 32),
        resolved_fixup: read_u64(&bytes, 40),
    })
}

fn read_fixup_table(memory: &GuestMemory) -> Result<[PartialDispatchFixupEntry; 2], Error> {
    let mut bytes = [0_u8; FIXUP_TABLE_BYTES];
    memory.read(PARTIAL_FIXUP_TABLE_ADDR, &mut bytes)?;
    Ok([
        PartialDispatchFixupEntry {
            fault_rip: read_u64(&bytes, 0),
            fixup_rip: read_u64(&bytes, 8),
            observation_addr: read_u64(&bytes, 16),
        },
        PartialDispatchFixupEntry {
            fault_rip: read_u64(&bytes, 24),
            fixup_rip: read_u64(&bytes, 32),
            observation_addr: read_u64(&bytes, 40),
        },
    ])
}

fn read_terminal_frame(memory: &GuestMemory) -> Result<PartialDispatchTerminalFrame, Error> {
    let start = GuestPhysAddr::new(PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES);
    let mut bytes = [0_u8; PRIVILEGE_FRAME_BYTES as usize];
    memory.read(start, &mut bytes)?;
    Ok(PartialDispatchTerminalFrame {
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
            .expect("fixed partial-dispatch field is eight bytes"),
    )
}

fn write_u64(memory: &mut GuestMemory, address: GuestPhysAddr, value: u64) -> Result<(), Error> {
    memory.write(address, &value.to_le_bytes())
}

fn validate_runtime_state(result: &PartialDispatchGuestResult, terminal_return_rip: u64) -> Result<(), Error> {
    let expected_returns = [
        4,
        2,
        2,
        1,
        PARTIAL_EINVAL,
        PARTIAL_EINVAL,
        0,
        0,
        PARTIAL_ENOSYS,
    ];
    let expected_source_fault_destination = [
        PARTIAL_SOURCE_FAULT_BYTES[0],
        PARTIAL_SOURCE_FAULT_BYTES[1],
        0,
        0,
    ];
    let expected_destination_fault_destination = [
        PARTIAL_DEST_FAULT_BYTES[0],
        PARTIAL_DEST_FAULT_BYTES[1],
        0,
        0,
    ];
    let expected_short_destination = [PARTIAL_SHORT_BYTES[0], 0, 0, 0];
    let expected_read_fault = PartialDispatchFaultObservation {
        cr2: PARTIAL_SOURCE_FAULT_ADDR,
        error_code: 0,
        rip: PARTIAL_READ_FAULT_RIP,
        cs: u64::from(KERNEL_CODE_SELECTOR),
        rflags: PAGE_FAULT_SAVED_RFLAGS,
        resolved_fixup: PARTIAL_COMMON_FIXUP_RIP,
    };
    let expected_write_fault = PartialDispatchFaultObservation {
        cr2: PARTIAL_DEST_FAULT_ADDR,
        error_code: 1 << 1,
        rip: PARTIAL_WRITE_FAULT_RIP,
        cs: u64::from(KERNEL_CODE_SELECTOR),
        rflags: PAGE_FAULT_SAVED_RFLAGS,
        resolved_fixup: PARTIAL_COMMON_FIXUP_RIP,
    };
    let expected_frame = PartialDispatchTerminalFrame {
        rip: terminal_return_rip,
        cs: u64::from(PRIVILEGE_USER_CODE_SELECTOR),
        rflags: X86_RFLAGS_RESERVED | X86_RFLAGS_IF,
        rsp: PRIVILEGE_USER_STACK,
        ss: u64::from(PRIVILEGE_USER_DATA_SELECTOR),
    };
    let mappings_valid = result.user_page_ptes.len() == USER_PAGES.len()
        && result.user_page_ptes.iter().zip(USER_PAGES).all(
            |((address, pte), (expected_address, present))| {
                *address == expected_address
                    && pte & (X86_PAGE_WRITE | X86_PAGE_USER) == (X86_PAGE_WRITE | X86_PAGE_USER)
                    && (pte & X86_PAGE_PRESENT != 0) == present
            },
        );

    if result.returns != expected_returns
        || result.good_destination != PARTIAL_GOOD_BYTES
        || result.source_fault_destination != expected_source_fault_destination
        || result.destination_fault_destination != expected_destination_fault_destination
        || result.short_destination != expected_short_destination
        || result.byte_destination != PARTIAL_BYTE_VALUE
        || result.read_fault != expected_read_fault
        || result.write_fault != expected_write_fault
        || result.fixup_entries != expected_fixup_entries()
        || !mappings_valid
        || result.service_pte & X86_PAGE_USER != 0
        || result.fault_handler_pte & X86_PAGE_USER != 0
        || result.fault_metadata_pte & X86_PAGE_USER != 0
        || result.terminal_frame != expected_frame
        || result.terminal_rsp != PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES
        || result.terminal_cs != KERNEL_CODE_SELECTOR
        || result.terminal_rflags & X86_RFLAGS_RESERVED != X86_RFLAGS_RESERVED
        || result.terminal_rflags & X86_RFLAGS_IF != 0
        || result.final_cr2 != PARTIAL_DEST_FAULT_ADDR
        || result.msrs[0] & EFER_SYSCALL_ENABLE != EFER_SYSCALL_ENABLE
        || result.msrs[1] != SYSCALL_STAR_VALUE
        || result.msrs[2] != SYSCALL_LSTAR_VALUE
        || result.msrs[3] != SYSCALL_SFMASK_VALUE
        || result.report.exit() != VcpuExit::Hlt
        || result.report.rip() != PRIVILEGE_TERMINAL_HANDLER.get() + 5
        || result.report.rflags() & X86_RFLAGS_RESERVED != X86_RFLAGS_RESERVED
    {
        return Err(verification_error(
            "partial-copy dispatcher architectural state",
            format!(
                "returns={:?} good={:?} source_fault={:?} destination_fault={:?} short={:?} byte={:#x} read_fault={:?} write_fault={:?} fixups={:?} terminal={:?}/{:#x}/{:#x}/{:#x} cr2={:#x} msrs={:#x?} report={}",
                result.returns,
                result.good_destination,
                result.source_fault_destination,
                result.destination_fault_destination,
                result.short_destination,
                result.byte_destination,
                result.read_fault,
                result.write_fault,
                result.fixup_entries,
                result.terminal_frame,
                result.terminal_rsp,
                result.terminal_cs,
                result.terminal_rflags,
                result.final_cr2,
                result.msrs,
                result.report
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
    fn appended_range_service_preserves_integrated_dispatcher_fault_offsets() {
        assert_eq!(&PARTIAL_DISPATCHER_BYTES[13..19], &[0x48, 0x83, 0xf8, 0x00, 0x74, 0x13]);
        assert_eq!(&PARTIAL_DISPATCHER_BYTES[19..25], &[0x48, 0x83, 0xf8, 0x01, 0x74, 0x34]);
        assert_eq!(&PARTIAL_DISPATCHER_BYTES[25..31], &[0x48, 0x83, 0xf8, 0x02, 0x74, 0x48]);
        assert_eq!(&PARTIAL_DISPATCHER_BYTES[38..43], &[0x0f, 0xb6, 0x07, 0x88, 0x06]);
        assert_eq!(SYSCALL_KERNEL_ENTRY.get() + 38, 0x12026);
        assert_eq!(SYSCALL_KERNEL_ENTRY.get() + 41, 0x12029);
        assert_eq!(SYSCALL_KERNEL_ENTRY.get() + 51, 0x12033);
        assert_eq!(SYSCALL_KERNEL_ENTRY.get() + 64, 0x12040);
        assert_eq!(SYSCALL_KERNEL_ENTRY.get() + 84, 0x12054);
    }

    #[test]
    fn range_service_uses_one_load_store_pair_and_progress_preserving_fixup() {
        assert_eq!(&PARTIAL_DISPATCHER_BYTES[117..120], &[0x4d, 0x39, 0xc0]);
        assert_eq!(&PARTIAL_DISPATCHER_BYTES[120..125], &[0x42, 0x0f, 0xb6, 0x04, 0x07]);
        assert_eq!(&PARTIAL_DISPATCHER_BYTES[125..129], &[0x42, 0x88, 0x04, 0x06]);
        assert_eq!(PARTIAL_READ_FAULT_RIP, 0x12078);
        assert_eq!(PARTIAL_WRITE_FAULT_RIP, 0x1207d);
        assert_eq!(PARTIAL_COMMON_FIXUP_RIP, 0x12098);
        assert_eq!(PARTIAL_COPY_MAX_LEN, 4);
    }

    #[test]
    fn ring3_program_uses_variable_lengths_and_no_direct_debug_output() {
        let code = build_user_guest();
        let syscall_count = code.windows(2).filter(|w| *w == [0x0f, 0x05]).count();
        assert_eq!(syscall_count, 9);
        assert!(!code.windows(2).any(|w| w == [0xe6, 0xe9]));
        assert_eq!(&code[code.len() - 2..], &[0xcd, 0x81]);
    }

    #[test]
    fn range_fixup_table_has_only_the_two_reusable_loop_fault_sites() {
        let entries = expected_fixup_entries();
        assert_eq!(entries[0].fault_rip(), PARTIAL_READ_FAULT_RIP);
        assert_eq!(entries[1].fault_rip(), PARTIAL_WRITE_FAULT_RIP);
        assert_eq!(entries[0].fixup_rip(), PARTIAL_COMMON_FIXUP_RIP);
        assert_eq!(entries[1].fixup_rip(), PARTIAL_COMMON_FIXUP_RIP);
        assert_eq!(encoded_fixup_table().len(), 48);
    }
}
