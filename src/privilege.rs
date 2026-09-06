use crate::config::VmConfig;
use crate::error::{Error, VmExitError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{
    LongModeBootLayout, LongModeConfigurationError, LONG_MODE_IDENTITY_MAP_SIZE,
    LONG_MODE_PAGE_SIZE, LONG_MODE_PDPT_ADDR, LONG_MODE_PD_ADDR, LONG_MODE_PML4_ADDR,
};
use crate::memory::{GuestMemory, GuestMemoryRegion, GuestPhysAddr};
use crate::portio::PortIoBus;
use crate::vcpu::{PortIoExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::fmt;

pub const PRIVILEGE_GDT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x5000);
pub const PRIVILEGE_IDT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x6000);
pub const PRIVILEGE_TSS_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x7000);
pub const PRIVILEGE_PT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x8000);
pub const PRIVILEGE_TABLE_END: GuestPhysAddr = GuestPhysAddr::new(0x9000);
pub const PRIVILEGE_OBSERVATION_ADDR: GuestPhysAddr = GuestPhysAddr::new(0xa000);
pub const PRIVILEGE_KERNEL_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
pub const PRIVILEGE_USER_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_1000);
pub const PRIVILEGE_RETURN_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_2000);
pub const PRIVILEGE_TERMINAL_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_3000);
pub const PRIVILEGE_BOOT_STACK: u64 = 0x1f_f000;
pub const PRIVILEGE_TSS_RSP0: u64 = 0x1f_e000;
pub const PRIVILEGE_USER_STACK: u64 = 0x1f_d000;
pub const PRIVILEGE_USER_CODE_SELECTOR: u16 = 0x23;
pub const PRIVILEGE_USER_DATA_SELECTOR: u16 = 0x1b;
pub const PRIVILEGE_TSS_SELECTOR: u16 = 0x28;
pub const PRIVILEGE_RETURN_VECTOR: u8 = 0x80;
pub const PRIVILEGE_TERMINAL_VECTOR: u8 = 0x81;
pub const PRIVILEGE_USER_RETURN_RIP: u64 = 0x1_1025;
pub const PRIVILEGE_TERMINAL_RIP: u64 = 0x1_3005;
pub const PRIVILEGE_PROOF: &[u8; 2] = b"KD";

const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITABLE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
#[cfg(test)]
const X86_PAGE_ACCESSED: u64 = 1 << 5;
#[cfg(test)]
const X86_PAGE_DIRTY: u64 = 1 << 6;
const X86_RFLAGS_RESERVED: u64 = 1 << 1;
const X86_RFLAGS_IF: u64 = 1 << 9;
const X86_INTERRUPT_GATE_SIZE: u64 = 16;
const KERNEL_CODE_SELECTOR: u16 = 0x08;
#[cfg(test)]
const KERNEL_DATA_SELECTOR: u16 = 0x10;
const GDT_LIMIT: u16 = 55;
const IDT_LIMIT: u16 = 0x081f;
const TSS_LIMIT: u32 = 103;
const TSS_BYTES: usize = 104;
const TSS_IO_BITMAP_OFFSET: usize = 102;
const PRIVILEGE_EXIT_BUDGET: u32 = 3;
const PRIVILEGE_FRAME_BYTES: u64 = 5 * 8;

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

const USER_BYTES: [u8; 37] = [
    0x48, 0xbf, 0x00, 0xa0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs obs, rdi
    0x8c, 0xc8, // mov cs, ax
    0x66, 0x89, 0x07, // mov ax, [rdi]
    0x8c, 0xd0, // mov ss, ax
    0x66, 0x89, 0x47, 0x02, // mov ax, [rdi+2]
    0xcd, 0x80, // int 0x80
    0x8c, 0xc8, // mov cs, ax
    0x66, 0x89, 0x47, 0x04, // mov ax, [rdi+4]
    0x8c, 0xd0, // mov ss, ax
    0x66, 0x89, 0x47, 0x06, // mov ax, [rdi+6]
    0xcd, 0x81, // int 0x81
];

const RETURN_HANDLER_BYTES: [u8; 6] = [
    0xb0, b'K', // mov al, 'K'
    0xe6, 0xe9, // out al, 0xe9
    0x48, 0xcf, // iretq
];

const TERMINAL_HANDLER_BYTES: [u8; 5] = [
    0xb0, b'D', // mov al, 'D'
    0xe6, 0xe9, // out al, 0xe9
    0xf4, // hlt in ring 0
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivilegeConfigurationError {
    Boot(LongModeConfigurationError),
    AddressOutsideIdentityMap { role: &'static str, address: u64 },
    AddressOverlapsTables { role: &'static str, address: u64 },
    StackOutsideIdentityMap { role: &'static str, stack: u64 },
    KernelAddressSharesUserPage { role: &'static str, address: u64 },
}

impl fmt::Display for PrivilegeConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boot(error) => error.fmt(f),
            Self::AddressOutsideIdentityMap { role, address } => {
                write!(
                    f,
                    "{role} address {address:#x} is outside the 2 MiB privilege identity map"
                )
            }
            Self::AddressOverlapsTables { role, address } => write!(
                f,
                "{role} address {address:#x} overlaps privilege page-table/GDT/IDT/TSS storage"
            ),
            Self::StackOutsideIdentityMap { role, stack } => {
                write!(
                    f,
                    "{role} stack pointer {stack:#x} is outside the privilege identity map"
                )
            }
            Self::KernelAddressSharesUserPage { role, address } => write!(
                f,
                "{role} address {address:#x} shares a 4 KiB page that must be user-accessible"
            ),
        }
    }
}

impl std::error::Error for PrivilegeConfigurationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Boot(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LongModeConfigurationError> for PrivilegeConfigurationError {
    fn from(error: LongModeConfigurationError) -> Self {
        Self::Boot(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongModePrivilegeLayout {
    boot: LongModeBootLayout,
}

impl LongModePrivilegeLayout {
    pub fn new(memory: GuestMemoryRegion) -> Result<Self, PrivilegeConfigurationError> {
        let boot = LongModeBootLayout::new(memory, PRIVILEGE_KERNEL_ENTRY, PRIVILEGE_BOOT_STACK)?;
        for (role, address) in [
            ("kernel entry", PRIVILEGE_KERNEL_ENTRY.get()),
            ("user entry", PRIVILEGE_USER_ENTRY.get()),
            ("return handler", PRIVILEGE_RETURN_HANDLER.get()),
            ("terminal handler", PRIVILEGE_TERMINAL_HANDLER.get()),
            ("selector observation", PRIVILEGE_OBSERVATION_ADDR.get()),
        ] {
            validate_address(role, address)?;
        }
        for (role, stack) in [
            ("bootstrap", PRIVILEGE_BOOT_STACK),
            ("TSS RSP0", PRIVILEGE_TSS_RSP0),
            ("user", PRIVILEGE_USER_STACK),
        ] {
            if stack == 0 || stack > LONG_MODE_IDENTITY_MAP_SIZE {
                return Err(PrivilegeConfigurationError::StackOutsideIdentityMap { role, stack });
            }
        }
        for (role, address) in [
            ("kernel entry", PRIVILEGE_KERNEL_ENTRY.get()),
            ("return handler", PRIVILEGE_RETURN_HANDLER.get()),
            ("terminal handler", PRIVILEGE_TERMINAL_HANDLER.get()),
            ("bootstrap stack", PRIVILEGE_BOOT_STACK - 1),
            ("TSS kernel stack", PRIVILEGE_TSS_RSP0 - 1),
        ] {
            if is_user_page(address) {
                return Err(PrivilegeConfigurationError::KernelAddressSharesUserPage {
                    role,
                    address,
                });
            }
        }
        Ok(Self { boot })
    }

    #[must_use]
    pub const fn boot_layout(&self) -> &LongModeBootLayout {
        &self.boot
    }

    #[must_use]
    pub const fn gdt_base(&self) -> GuestPhysAddr {
        PRIVILEGE_GDT_ADDR
    }

    #[must_use]
    pub const fn gdt_limit(&self) -> u16 {
        GDT_LIMIT
    }

    #[must_use]
    pub const fn idt_base(&self) -> GuestPhysAddr {
        PRIVILEGE_IDT_ADDR
    }

    #[must_use]
    pub const fn idt_limit(&self) -> u16 {
        IDT_LIMIT
    }

    pub(crate) fn install_tables(&self, memory: &mut GuestMemory) -> Result<(), Error> {
        debug_assert_eq!(memory.region(), self.boot.memory());
        self.boot.install_page_tables(memory)?;

        let zero_page = [0_u8; LONG_MODE_PAGE_SIZE as usize];
        memory.write(PRIVILEGE_GDT_ADDR, &zero_page)?;
        memory.write(PRIVILEGE_IDT_ADDR, &zero_page)?;
        memory.write(PRIVILEGE_TSS_ADDR, &zero_page)?;
        memory.write(PRIVILEGE_PT_ADDR, &zero_page)?;

        install_privilege_page_tables(memory)?;
        memory.write(PRIVILEGE_GDT_ADDR, &gdt_bytes())?;
        memory.write(
            GuestPhysAddr::new(
                PRIVILEGE_IDT_ADDR.get()
                    + u64::from(PRIVILEGE_RETURN_VECTOR) * X86_INTERRUPT_GATE_SIZE,
            ),
            &encode_user_interrupt_gate(PRIVILEGE_RETURN_HANDLER.get()),
        )?;
        memory.write(
            GuestPhysAddr::new(
                PRIVILEGE_IDT_ADDR.get()
                    + u64::from(PRIVILEGE_TERMINAL_VECTOR) * X86_INTERRUPT_GATE_SIZE,
            ),
            &encode_user_interrupt_gate(PRIVILEGE_TERMINAL_HANDLER.get()),
        )?;
        memory.write(PRIVILEGE_TSS_ADDR, &tss_bytes())?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivilegeStackFrame {
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

impl PrivilegeStackFrame {
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
pub struct PrivilegeTransitionGuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
    user_selectors: [u16; 4],
    frame: PrivilegeStackFrame,
    terminal_rsp: u64,
    terminal_cs: u16,
    terminal_rflags: u64,
    tr_selector: u16,
    tr_base: u64,
    tr_limit: u32,
    tr_type: u8,
    tss_descriptor_access: u8,
    user_code_pte: u64,
    observation_pte: u64,
    user_stack_pte: u64,
    kernel_handler_pte: u64,
}

impl PrivilegeTransitionGuestResult {
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
    pub const fn frame(&self) -> PrivilegeStackFrame {
        self.frame
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
    pub const fn tr_selector(&self) -> u16 {
        self.tr_selector
    }
    #[must_use]
    pub const fn tr_base(&self) -> u64 {
        self.tr_base
    }
    #[must_use]
    pub const fn tr_limit(&self) -> u32 {
        self.tr_limit
    }
    #[must_use]
    pub const fn tr_type(&self) -> u8 {
        self.tr_type
    }
    #[must_use]
    pub const fn tss_descriptor_access(&self) -> u8 {
        self.tss_descriptor_access
    }
    #[must_use]
    pub const fn user_code_pte(&self) -> u64 {
        self.user_code_pte
    }
    #[must_use]
    pub const fn observation_pte(&self) -> u64 {
        self.observation_pte
    }
    #[must_use]
    pub const fn user_stack_pte(&self) -> u64 {
        self.user_stack_pte
    }
    #[must_use]
    pub const fn kernel_handler_pte(&self) -> u64 {
        self.kernel_handler_pte
    }
}

pub fn run_privilege_transition_guest(
    config: VmConfig,
) -> Result<PrivilegeTransitionGuestResult, Error> {
    let kernel = FlatGuestImage::new(
        PRIVILEGE_KERNEL_ENTRY,
        PRIVILEGE_KERNEL_ENTRY,
        &KERNEL_BOOT_BYTES,
    )?;
    let user = FlatGuestImage::new(PRIVILEGE_USER_ENTRY, PRIVILEGE_USER_ENTRY, &USER_BYTES)?;
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
        .expect("fixed bounded ring3/TSS layout remains valid");
    layout.install_tables(&mut memory)?;
    kernel.load(&mut memory)?;
    user.load(&mut memory)?;
    return_handler.load(&mut memory)?;
    terminal_handler.load(&mut memory)?;
    memory.write(PRIVILEGE_OBSERVATION_ADDR, &[0; 8])?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_privilege(&layout)?;
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, PRIVILEGE_EXIT_BUDGET)?;
    if execution.io_exits().len() != PRIVILEGE_PROOF.len() {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "ring3 privilege-transition proof output count",
            expected_reason: crate::vcpu::VcpuExit::Io.reason(),
            actual_reason: execution.report().exit().reason(),
        }));
    }

    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != PRIVILEGE_PROOF {
        return Err(verification_error(
            "ring3 privilege-transition proof",
            format!("expected {PRIVILEGE_PROOF:?}, got {proof:?}"),
        ));
    }

    let regs = vcpu.registers()?;
    let register_snapshot = vcpu.capture_register_snapshot()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let tr = special.tr();
    let guest_memory = vm
        .guest_memory()
        .expect("registered privilege-transition memory remains VM-owned");
    let user_selectors = read_user_selectors(guest_memory)?;
    let frame = read_stack_frame(guest_memory)?;
    let tss_descriptor_access = read_byte(
        guest_memory,
        GuestPhysAddr::new(PRIVILEGE_GDT_ADDR.get() + 45),
    )?;
    let user_code_pte = read_pte(guest_memory, PRIVILEGE_USER_ENTRY.get())?;
    let observation_pte = read_pte(guest_memory, PRIVILEGE_OBSERVATION_ADDR.get())?;
    let user_stack_pte = read_pte(guest_memory, PRIVILEGE_USER_STACK - 1)?;
    let kernel_handler_pte = read_pte(guest_memory, PRIVILEGE_TERMINAL_HANDLER.get())?;

    validate_runtime_state(
        user_selectors,
        frame,
        register_snapshot.rsp(),
        regs.rflags,
        special.cs().selector(),
        tr.selector(),
        tr.base(),
        tr.limit(),
        tr.segment_type(),
        tr.present(),
        tr.s(),
        tr.unusable(),
        special.gdt().base(),
        special.gdt().limit(),
        special.idt().base(),
        special.idt().limit(),
        tss_descriptor_access,
        user_code_pte,
        observation_pte,
        user_stack_pte,
        kernel_handler_pte,
    )?;

    Ok(PrivilegeTransitionGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
        user_selectors,
        frame,
        terminal_rsp: register_snapshot.rsp(),
        terminal_cs: special.cs().selector(),
        terminal_rflags: regs.rflags,
        tr_selector: tr.selector(),
        tr_base: tr.base(),
        tr_limit: tr.limit(),
        tr_type: tr.segment_type(),
        tss_descriptor_access,
        user_code_pte,
        observation_pte,
        user_stack_pte,
        kernel_handler_pte,
    })
}

fn install_privilege_page_tables(memory: &mut GuestMemory) -> Result<(), Error> {
    write_u64(memory, LONG_MODE_PML4_ADDR, LONG_MODE_PDPT_ADDR.get() | 0x7)?;
    write_u64(memory, LONG_MODE_PDPT_ADDR, LONG_MODE_PD_ADDR.get() | 0x7)?;
    write_u64(memory, LONG_MODE_PD_ADDR, PRIVILEGE_PT_ADDR.get() | 0x7)?;
    for index in 0..512_u64 {
        let address = index * LONG_MODE_PAGE_SIZE;
        let flags = X86_PAGE_PRESENT
            | X86_PAGE_WRITABLE
            | if is_user_page(address) {
                X86_PAGE_USER
            } else {
                0
            };
        write_u64(
            memory,
            GuestPhysAddr::new(PRIVILEGE_PT_ADDR.get() + index * 8),
            address | flags,
        )?;
    }
    Ok(())
}

fn gdt_bytes() -> [u8; 56] {
    let mut gdt = [0_u8; 56];
    gdt[8..16].copy_from_slice(&[0xff, 0xff, 0, 0, 0, 0x9b, 0xaf, 0]);
    gdt[16..24].copy_from_slice(&[0xff, 0xff, 0, 0, 0, 0x93, 0x8f, 0]);
    gdt[24..32].copy_from_slice(&[0xff, 0xff, 0, 0, 0, 0xf3, 0x8f, 0]);
    gdt[32..40].copy_from_slice(&[0xff, 0xff, 0, 0, 0, 0xfb, 0xaf, 0]);
    gdt[40..56].copy_from_slice(&encode_tss_descriptor(PRIVILEGE_TSS_ADDR.get()));
    gdt
}

fn encode_tss_descriptor(base: u64) -> [u8; 16] {
    let mut descriptor = [0_u8; 16];
    descriptor[0..2].copy_from_slice(&(TSS_LIMIT as u16).to_le_bytes());
    descriptor[2..4].copy_from_slice(&(base as u16).to_le_bytes());
    descriptor[4] = (base >> 16) as u8;
    descriptor[5] = 0x89; // present, DPL0, available 64-bit TSS
    descriptor[6] = ((TSS_LIMIT >> 16) & 0x0f) as u8;
    descriptor[7] = (base >> 24) as u8;
    descriptor[8..12].copy_from_slice(&((base >> 32) as u32).to_le_bytes());
    descriptor
}

fn encode_user_interrupt_gate(handler: u64) -> [u8; 16] {
    let mut gate = [0_u8; 16];
    gate[0..2].copy_from_slice(&(handler as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&KERNEL_CODE_SELECTOR.to_le_bytes());
    gate[4] = 0;
    gate[5] = 0xee; // present DPL3 64-bit interrupt gate
    gate[6..8].copy_from_slice(&((handler >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((handler >> 32) as u32).to_le_bytes());
    gate
}

fn tss_bytes() -> [u8; TSS_BYTES] {
    let mut tss = [0_u8; TSS_BYTES];
    tss[4..12].copy_from_slice(&PRIVILEGE_TSS_RSP0.to_le_bytes());
    tss[TSS_IO_BITMAP_OFFSET..TSS_IO_BITMAP_OFFSET + 2]
        .copy_from_slice(&(TSS_BYTES as u16).to_le_bytes());
    tss
}

fn validate_address(role: &'static str, address: u64) -> Result<(), PrivilegeConfigurationError> {
    if address >= LONG_MODE_IDENTITY_MAP_SIZE {
        return Err(PrivilegeConfigurationError::AddressOutsideIdentityMap { role, address });
    }
    if address >= LONG_MODE_PML4_ADDR.get() && address < PRIVILEGE_TABLE_END.get() {
        return Err(PrivilegeConfigurationError::AddressOverlapsTables { role, address });
    }
    Ok(())
}

const fn page_start(address: u64) -> u64 {
    address & !(LONG_MODE_PAGE_SIZE - 1)
}

const fn is_user_page(address: u64) -> bool {
    let page = page_start(address);
    page == page_start(PRIVILEGE_USER_ENTRY.get())
        || page == page_start(PRIVILEGE_OBSERVATION_ADDR.get())
        || page == page_start(PRIVILEGE_USER_STACK - 1)
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

fn read_stack_frame(memory: &GuestMemory) -> Result<PrivilegeStackFrame, Error> {
    let start = GuestPhysAddr::new(PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES);
    let mut bytes = [0_u8; PRIVILEGE_FRAME_BYTES as usize];
    memory.read(start, &mut bytes)?;
    let value = |offset: usize| {
        u64::from_le_bytes(
            bytes[offset..offset + 8]
                .try_into()
                .expect("8-byte frame field"),
        )
    };
    Ok(PrivilegeStackFrame {
        rip: value(0),
        cs: value(8),
        rflags: value(16),
        rsp: value(24),
        ss: value(32),
    })
}

fn read_pte(memory: &GuestMemory, address: u64) -> Result<u64, Error> {
    let index = page_start(address) / LONG_MODE_PAGE_SIZE;
    let mut bytes = [0_u8; 8];
    memory.read(
        GuestPhysAddr::new(PRIVILEGE_PT_ADDR.get() + index * 8),
        &mut bytes,
    )?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_byte(memory: &GuestMemory, address: GuestPhysAddr) -> Result<u8, Error> {
    let mut byte = [0_u8; 1];
    memory.read(address, &mut byte)?;
    Ok(byte[0])
}

fn write_u64(memory: &mut GuestMemory, address: GuestPhysAddr, value: u64) -> Result<(), Error> {
    memory.write(address, &value.to_le_bytes())
}

#[allow(clippy::too_many_arguments)]
fn validate_runtime_state(
    selectors: [u16; 4],
    frame: PrivilegeStackFrame,
    terminal_rsp: u64,
    terminal_rflags: u64,
    terminal_cs: u16,
    tr_selector: u16,
    tr_base: u64,
    tr_limit: u32,
    tr_type: u8,
    tr_present: u8,
    tr_s: u8,
    tr_unusable: u8,
    gdt_base: u64,
    gdt_limit: u16,
    idt_base: u64,
    idt_limit: u16,
    tss_descriptor_access: u8,
    user_code_pte: u64,
    observation_pte: u64,
    user_stack_pte: u64,
    kernel_handler_pte: u64,
) -> Result<(), Error> {
    if selectors
        != [
            PRIVILEGE_USER_CODE_SELECTOR,
            PRIVILEGE_USER_DATA_SELECTOR,
            PRIVILEGE_USER_CODE_SELECTOR,
            PRIVILEGE_USER_DATA_SELECTOR,
        ]
        || frame.rip != PRIVILEGE_USER_RETURN_RIP
        || frame.cs != u64::from(PRIVILEGE_USER_CODE_SELECTOR)
        || frame.rflags != X86_RFLAGS_RESERVED | X86_RFLAGS_IF
        || frame.rsp != PRIVILEGE_USER_STACK
        || frame.ss != u64::from(PRIVILEGE_USER_DATA_SELECTOR)
        || terminal_rsp != PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES
        || terminal_cs != KERNEL_CODE_SELECTOR
        || terminal_rflags & X86_RFLAGS_RESERVED == 0
        || terminal_rflags & X86_RFLAGS_IF != 0
        || tr_selector != PRIVILEGE_TSS_SELECTOR
        || tr_base != PRIVILEGE_TSS_ADDR.get()
        || tr_limit != TSS_LIMIT
        || tr_type != 0x0b
        || tr_present != 1
        || tr_s != 0
        || tr_unusable != 0
        || gdt_base != PRIVILEGE_GDT_ADDR.get()
        || gdt_limit != GDT_LIMIT
        || idt_base != PRIVILEGE_IDT_ADDR.get()
        || idt_limit != IDT_LIMIT
        || tss_descriptor_access != 0x8b
        || user_code_pte & X86_PAGE_USER == 0
        || observation_pte & X86_PAGE_USER == 0
        || user_stack_pte & X86_PAGE_USER == 0
        || kernel_handler_pte & X86_PAGE_USER != 0
    {
        return Err(verification_error(
            "ring3 privilege-transition architectural state",
            format!(
                "selectors={selectors:?} frame={frame:?} terminal_rsp={terminal_rsp:#x} terminal_cs={terminal_cs:#x} terminal_rflags={terminal_rflags:#x} tr={tr_selector:#x}/{tr_base:#x}/{tr_limit:#x}/type{tr_type:#x} gdt={gdt_base:#x}/{gdt_limit:#x} idt={idt_base:#x}/{idt_limit:#x} tss_access={tss_descriptor_access:#x} ptes={user_code_pte:#x},{observation_pte:#x},{user_stack_pte:#x},{kernel_handler_pte:#x}"
            ),
        ));
    }
    Ok(())
}

fn verification_error(operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(crate::error::HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation,
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> LongModePrivilegeLayout {
        LongModePrivilegeLayout::new(
            GuestMemoryRegion::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn installs_ring3_gdt_dpl3_gates_tss_and_user_page_permissions() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        layout().install_tables(&mut memory).unwrap();

        let mut gdt = [0_u8; 56];
        memory.read(PRIVILEGE_GDT_ADDR, &mut gdt).unwrap();
        assert_eq!(gdt[13], 0x9b);
        assert_eq!(gdt[21], 0x93);
        assert_eq!(gdt[29], 0xf3);
        assert_eq!(gdt[37], 0xfb);
        assert_eq!(gdt[45], 0x89);
        assert_eq!(
            &gdt[40..56],
            &encode_tss_descriptor(PRIVILEGE_TSS_ADDR.get())
        );

        let mut first_gate = [0_u8; 16];
        memory
            .read(
                GuestPhysAddr::new(PRIVILEGE_IDT_ADDR.get() + 0x80 * 16),
                &mut first_gate,
            )
            .unwrap();
        assert_eq!(first_gate[5], 0xee);
        assert_eq!(
            first_gate,
            encode_user_interrupt_gate(PRIVILEGE_RETURN_HANDLER.get())
        );

        let mut second_gate = [0_u8; 16];
        memory
            .read(
                GuestPhysAddr::new(PRIVILEGE_IDT_ADDR.get() + 0x81 * 16),
                &mut second_gate,
            )
            .unwrap();
        assert_eq!(
            second_gate,
            encode_user_interrupt_gate(PRIVILEGE_TERMINAL_HANDLER.get())
        );

        let mut tss = [0_u8; TSS_BYTES];
        memory.read(PRIVILEGE_TSS_ADDR, &mut tss).unwrap();
        assert_eq!(
            u64::from_le_bytes(tss[4..12].try_into().unwrap()),
            PRIVILEGE_TSS_RSP0
        );
        assert_eq!(u16::from_le_bytes(tss[102..104].try_into().unwrap()), 104);

        let user_code = read_pte(&memory, PRIVILEGE_USER_ENTRY.get()).unwrap();
        let observation = read_pte(&memory, PRIVILEGE_OBSERVATION_ADDR.get()).unwrap();
        let user_stack = read_pte(&memory, PRIVILEGE_USER_STACK - 1).unwrap();
        let kernel = read_pte(&memory, PRIVILEGE_TERMINAL_HANDLER.get()).unwrap();
        assert_ne!(user_code & X86_PAGE_USER, 0);
        assert_ne!(observation & X86_PAGE_USER, 0);
        assert_ne!(user_stack & X86_PAGE_USER, 0);
        assert_eq!(kernel & X86_PAGE_USER, 0);
    }

    #[test]
    fn guest_machine_code_uses_ltr_iretq_and_two_user_callable_traps() {
        assert!(KERNEL_BOOT_BYTES
            .windows(3)
            .any(|window| window == [0x0f, 0x00, 0xd8]));
        assert_eq!(
            &KERNEL_BOOT_BYTES[KERNEL_BOOT_BYTES.len() - 2..],
            &[0x48, 0xcf]
        );
        assert!(USER_BYTES.windows(2).any(|window| window == [0xcd, 0x80]));
        assert!(USER_BYTES.windows(2).any(|window| window == [0xcd, 0x81]));
        assert_eq!(
            PRIVILEGE_USER_ENTRY.get() + USER_BYTES.len() as u64,
            PRIVILEGE_USER_RETURN_RIP
        );
        assert_eq!(
            RETURN_HANDLER_BYTES[RETURN_HANDLER_BYTES.len() - 2..],
            [0x48, 0xcf]
        );
        assert_eq!(
            TERMINAL_HANDLER_BYTES[TERMINAL_HANDLER_BYTES.len() - 1],
            0xf4
        );
    }

    #[test]
    fn user_and_supervisor_pages_are_distinct_and_observation_writes_can_set_ad_bits() {
        assert_ne!(
            page_start(PRIVILEGE_USER_ENTRY.get()),
            page_start(PRIVILEGE_KERNEL_ENTRY.get())
        );
        assert_ne!(
            page_start(PRIVILEGE_USER_STACK - 1),
            page_start(PRIVILEGE_TSS_RSP0 - 1)
        );
        assert_eq!(X86_PAGE_ACCESSED, 0x20);
        assert_eq!(X86_PAGE_DIRTY, 0x40);
        assert_eq!(KERNEL_DATA_SELECTOR, 0x10);
    }
}
