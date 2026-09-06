use super::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::error::{Error, HostEnvironmentError};
use crate::interrupt::{LongModeInterruptLayout, LONG_MODE_INTERRUPT_IDT_ADDR};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{
    LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS,
    LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PML4_ADDR,
};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::mmio::long_mode::{LongModeMmioBootLayout, LongModeMmioPageMapping};
use crate::portio::two_vcpu_init_sipi_fixture::{
    AP_LONG_MODE_CODE_SELECTOR, AP_LONG_MODE_DATA_SELECTOR, AP_LONG_MODE_GDT,
    AP_LONG_MODE_GDT_LIMIT, AP_LONG_MODE_STACK, FIRST_VCPU_ID, LAPIC_GPA, LAPIC_VIRTUAL_PAGE,
    SECOND_VCPU_ID,
};
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit};
use std::io;
use std::sync::mpsc;

const RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const BSP_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
const BSP_STACK: u64 = 0x1f_f000;
const AP_TRAMPOLINE: GuestPhysAddr = GuestPhysAddr::new(0x8000);
const AP_LONG_MODE_GDTR: GuestPhysAddr = GuestPhysAddr::new(0x7020);
const AP_TLB_IDTR: GuestPhysAddr = GuestPhysAddr::new(0x7040);
const SHOOTDOWN_ACK: GuestPhysAddr = GuestPhysAddr::new(0x9000);
const KVM_MP_STATE_RUNNABLE: u32 = 0;
const KVM_MP_STATE_UNINITIALIZED: u32 = 1;
const SIPI_CS_SELECTOR: u16 = 0x0800;
const SIPI_CS_BASE: u64 = 0x8000;
const X86_CR0_PROTECTED_MODE_ENABLE: u64 = 1;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const X86_RFLAGS_INTERRUPT_ENABLE: u64 = 1 << 9;
const PAGE_TABLE_ENTRY_FLAGS: u64 = 0x3;
const PAGE_TABLE_ENTRY_ACCESSED: u64 = 1 << 5;
#[cfg(test)]
const PAGE_TABLE_ENTRY_DIRTY: u64 = 1 << 6;

pub const TLB_TARGET_VIRTUAL_PAGE: u64 = 0x50_1000;
pub const TLB_TARGET_PTE: GuestPhysAddr = GuestPhysAddr::new(0x4808);
pub const TLB_PAGE_A: GuestPhysAddr = GuestPhysAddr::new(0x1_8000);
pub const TLB_PAGE_B: GuestPhysAddr = GuestPhysAddr::new(0x1_9000);
pub const TLB_PAGE_A_VALUE: u8 = b'A';
pub const TLB_PAGE_B_VALUE: u8 = b'B';
pub const TLB_SHOOTDOWN_VECTOR: u8 = 0x54;
pub const TLB_SHOOTDOWN_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_4000);
pub const TLB_SHOOTDOWN_IDT_LIMIT: u16 = 0x054f;
pub const TLB_FINAL_PTE: u64 = 0x1_9000 | PAGE_TABLE_ENTRY_FLAGS | PAGE_TABLE_ENTRY_ACCESSED;
pub const BSP_TLB_PROOF: &[u8; 8] = b"0IDSPXAD";
pub const AP_TLB_PROOF: &[u8; 6] = b"ALRIBD";

const AP_LONG_MODE_GDT_BYTES: [u8; 24] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x9a, 0xaf, 0x00,
    0xff, 0xff, 0x00, 0x00, 0x00, 0x92, 0xcf, 0x00,
];
const AP_LONG_MODE_GDTR_BYTES: [u8; 6] = [0x17, 0x00, 0x00, 0x70, 0x00, 0x00];
const AP_TLB_IDTR_BYTES: [u8; 10] = [0x4f, 0x05, 0x00, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

// Generated as 64-bit code at VMA 0x10000. The first half preserves the integrated BSP
// INIT/SIPI sequence. After userspace has observed AP readiness, the BSP guest mutates the shared
// alias PTE from page A to page B, orders that store with MFENCE, then sends vector 0x54 to APIC ID1.
// It accepts completion only after the AP handler has acknowledged the shootdown.
#[rustfmt::skip]
const BSP_GUEST_BYTES: [u8; 171] = [
    0xfa, 0x48, 0xbb, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0xb0,
    0x30, 0xe6, 0xe9, 0xc7, 0x83, 0x10, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x00, 0xc5, 0x00, 0x00, 0xb0,
    0x49, 0xe6, 0xe9, 0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x00, 0x85, 0x00,
    0x00, 0xb0, 0x44, 0xe6, 0xe9, 0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x08,
    0x06, 0x00, 0x00, 0xb0, 0x53, 0xe6, 0xe9, 0x48, 0xb9, 0x08, 0x48, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xb8, 0x03, 0x90, 0x01, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x48, 0x89, 0x01, 0x0f, 0xae, 0xf0, 0xb0, 0x50, 0xe6,
    0xe9, 0xc7, 0x83, 0x10, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0xc7,
    0x83, 0x00, 0x03, 0x00, 0x00, 0x54, 0x00, 0x00, 0x00, 0xb0, 0x58, 0xe6,
    0xe9, 0x48, 0xbe, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xba,
    0x00, 0x00, 0x00, 0x08, 0x80, 0x3e, 0x01, 0x74, 0x08, 0xf3, 0x90, 0xff,
    0xca, 0x75, 0xf5, 0xeb, 0x11, 0x31, 0xc0, 0x86, 0x06, 0x3c, 0x01, 0x75,
    0x09, 0xb0, 0x41, 0xe6, 0xe9, 0xb0, 0x44, 0xe6, 0xe9, 0xf4, 0xb0, 0x46,
    0xe6, 0xe9, 0xf4,
];

// The first 73 bytes are byte-for-byte identical to the integrated SIPI real-mode-to-long-mode
// transition. The suffix primes VA 0x501000 while it maps page A, reports R under CLI, then waits
// for the remote shootdown. After vector 0x54 returns, the same VA must read page B before B,D.
#[rustfmt::skip]
const AP_GUEST_BYTES: [u8; 161] = [
    0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0, 0xb0, 0x41, 0xe6,
    0xe9, 0x0f, 0x01, 0x16, 0x20, 0x70, 0x0f, 0x20, 0xe0, 0x66, 0x83, 0xc8,
    0x20, 0x0f, 0x22, 0xe0, 0x66, 0xb8, 0x00, 0x10, 0x00, 0x00, 0x0f, 0x22,
    0xd8, 0x66, 0xb9, 0x80, 0x00, 0x00, 0xc0, 0x0f, 0x32, 0x66, 0x0d, 0x00,
    0x01, 0x00, 0x00, 0x0f, 0x30, 0x0f, 0x20, 0xc0, 0x66, 0x0d, 0x01, 0x00,
    0x00, 0x80, 0x0f, 0x22, 0xc0, 0x66, 0xea, 0x49, 0x80, 0x00, 0x00, 0x08,
    0x00,
    0x66, 0xb8, 0x10, 0x00, 0x8e, 0xd0, 0x8e, 0xd8, 0x8e, 0xc0, 0x48, 0xc7,
    0xc4, 0x00, 0xf0, 0x1e, 0x00, 0xb0, 0x4c, 0xe6, 0xe9, 0x48, 0xc7, 0xc3,
    0x00, 0x00, 0x50, 0x00, 0xc7, 0x83, 0xf0, 0x00, 0x00, 0x00, 0xff, 0x01,
    0x00, 0x00, 0x0f, 0x01, 0x1c, 0x25, 0x40, 0x70, 0x00, 0x00, 0x48, 0xbe,
    0x00, 0x10, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x8a, 0x06, 0x3c, 0x41,
    0x75, 0x15, 0xb0, 0x52, 0xe6, 0xe9, 0xfb, 0xf4, 0x8a, 0x06, 0x3c, 0x42,
    0x75, 0x09, 0xb0, 0x42, 0xe6, 0xe9, 0xb0, 0x44, 0xe6, 0xe9, 0xf4, 0xb0,
    0x46, 0xe6, 0xe9, 0xf4,
];

// INVLPG is deliberately before the observable I byte. Reaching I proves that the guest actually
// executed the invalidation instruction before publishing the shootdown acknowledgement.
#[rustfmt::skip]
const AP_HANDLER_BYTES: [u8; 32] = [
    0x0f, 0x01, 0x3e, 0x48, 0xb9, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0xc6, 0x01, 0x01, 0xb0, 0x49, 0xe6, 0xe9, 0xc7, 0x83, 0xb0, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xcf,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlbShootdownState {
    initial_ap_mp_state: u32,
    startup_rip: u64,
    startup_cs_selector: u16,
    startup_cs_base: u64,
    ready_rflags: u64,
    completion_rflags: u64,
    rsp: u64,
    cs_selector: u16,
    cs_long: u8,
    ss_selector: u16,
    gdt_base: u64,
    gdt_limit: u16,
    idt_base: u64,
    idt_limit: u16,
    cr0: u64,
    cr3: u64,
    cr4: u64,
    efer: u64,
}

impl TlbShootdownState {
    #[must_use]
    pub const fn initial_ap_mp_state(self) -> u32 {
        self.initial_ap_mp_state
    }
    #[must_use]
    pub const fn startup_rip(self) -> u64 {
        self.startup_rip
    }
    #[must_use]
    pub const fn startup_cs_selector(self) -> u16 {
        self.startup_cs_selector
    }
    #[must_use]
    pub const fn startup_cs_base(self) -> u64 {
        self.startup_cs_base
    }
    #[must_use]
    pub const fn ready_rflags(self) -> u64 {
        self.ready_rflags
    }
    #[must_use]
    pub const fn completion_rflags(self) -> u64 {
        self.completion_rflags
    }
    #[must_use]
    pub const fn rsp(self) -> u64 {
        self.rsp
    }
    #[must_use]
    pub const fn cs_selector(self) -> u16 {
        self.cs_selector
    }
    #[must_use]
    pub const fn cs_long(self) -> u8 {
        self.cs_long
    }
    #[must_use]
    pub const fn ss_selector(self) -> u16 {
        self.ss_selector
    }
    #[must_use]
    pub const fn gdt_base(self) -> u64 {
        self.gdt_base
    }
    #[must_use]
    pub const fn gdt_limit(self) -> u16 {
        self.gdt_limit
    }
    #[must_use]
    pub const fn idt_base(self) -> u64 {
        self.idt_base
    }
    #[must_use]
    pub const fn idt_limit(self) -> u16 {
        self.idt_limit
    }
    #[must_use]
    pub const fn cr0(self) -> u64 {
        self.cr0
    }
    #[must_use]
    pub const fn cr3(self) -> u64 {
        self.cr3
    }
    #[must_use]
    pub const fn cr4(self) -> u64 {
        self.cr4
    }
    #[must_use]
    pub const fn efer(self) -> u64 {
        self.efer
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuTlbShootdownResult {
    bsp_io_exits: Vec<PortIoExit>,
    ap_io_exits: Vec<PortIoExit>,
    bsp_proof: Vec<u8>,
    ap_proof: Vec<u8>,
    state: TlbShootdownState,
    final_pte: u64,
    final_ack: u8,
    page_a: u8,
    page_b: u8,
}

impl TwoVcpuTlbShootdownResult {
    #[must_use]
    pub fn bsp_io_exits(&self) -> &[PortIoExit] {
        &self.bsp_io_exits
    }
    #[must_use]
    pub fn ap_io_exits(&self) -> &[PortIoExit] {
        &self.ap_io_exits
    }
    #[must_use]
    pub fn bsp_proof(&self) -> &[u8] {
        &self.bsp_proof
    }
    #[must_use]
    pub fn ap_proof(&self) -> &[u8] {
        &self.ap_proof
    }
    #[must_use]
    pub const fn state(&self) -> TlbShootdownState {
        self.state
    }
    #[must_use]
    pub const fn final_pte(&self) -> u64 {
        self.final_pte
    }
    #[must_use]
    pub const fn final_ack(&self) -> u8 {
        self.final_ack
    }
    #[must_use]
    pub const fn page_a(&self) -> u8 {
        self.page_a
    }
    #[must_use]
    pub const fn page_b(&self) -> u8 {
        self.page_b
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ApStartupObservation {
    mp_state: u32,
    rip: u64,
    cs_selector: u16,
    cs_base: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApWorkerCommand {
    Continue,
    Abort,
}

#[derive(Debug)]
struct ApWorkerResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    ready_rflags: u64,
    completion_rflags: u64,
    startup: ApStartupObservation,
    rsp: u64,
    cs_selector: u16,
    cs_long: u8,
    ss_selector: u16,
    gdt_base: u64,
    gdt_limit: u16,
    idt_base: u64,
    idt_limit: u16,
    cr0: u64,
    cr3: u64,
    cr4: u64,
    efer: u64,
}

pub fn run_two_vcpu_tlb_shootdown() -> Result<TwoVcpuTlbShootdownResult, Error> {
    let bsp_image = FlatGuestImage::new(BSP_ENTRY, BSP_ENTRY, &BSP_GUEST_BYTES)?;
    let ap_image = FlatGuestImage::new(AP_TRAMPOLINE, AP_TRAMPOLINE, &AP_GUEST_BYTES)?;
    let handler = FlatGuestImage::new(
        TLB_SHOOTDOWN_HANDLER,
        TLB_SHOOTDOWN_HANDLER,
        &AP_HANDLER_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    backend.require_mp_state_capability()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(RAM_BASE, LONG_MODE_IDENTITY_MAP_SIZE)?;
    let bsp_layout = LongModeMmioBootLayout::with_device_mappings(
        memory.region(),
        bsp_image.entry(),
        BSP_STACK,
        vec![LongModeMmioPageMapping::new(LAPIC_VIRTUAL_PAGE, LAPIC_GPA)],
    )
    .expect("fixed TLB-shootdown BSP LAPIC mapping remains valid");
    let ap_interrupt_layout = LongModeInterruptLayout::new(
        memory.region(),
        AP_TRAMPOLINE,
        AP_LONG_MODE_STACK,
        TLB_SHOOTDOWN_VECTOR,
        TLB_SHOOTDOWN_HANDLER,
    )
    .expect("fixed TLB-shootdown AP interrupt layout remains valid");
    if ap_interrupt_layout.idt_base() != LONG_MODE_INTERRUPT_IDT_ADDR
        || ap_interrupt_layout.idt_limit() != TLB_SHOOTDOWN_IDT_LIMIT
    {
        return Err(verification_error(
            SECOND_VCPU_ID.get(),
            "TLB-shootdown AP IDT layout",
            format!(
                "unexpected IDT {:#x}/{:#x}",
                ap_interrupt_layout.idt_base().get(),
                ap_interrupt_layout.idt_limit()
            ),
        ));
    }
    ap_interrupt_layout.install_tables(&mut memory)?;
    bsp_layout.install_page_tables(&mut memory)?;
    write_u64(
        &mut memory,
        TLB_TARGET_PTE,
        TLB_PAGE_A.get() | PAGE_TABLE_ENTRY_FLAGS,
    )?;
    bsp_image.load(&mut memory)?;
    ap_image.load(&mut memory)?;
    handler.load(&mut memory)?;
    memory.write(AP_LONG_MODE_GDT, &AP_LONG_MODE_GDT_BYTES)?;
    memory.write(AP_LONG_MODE_GDTR, &AP_LONG_MODE_GDTR_BYTES)?;
    memory.write(AP_TLB_IDTR, &AP_TLB_IDTR_BYTES)?;
    memory.write(TLB_PAGE_A, &[TLB_PAGE_A_VALUE])?;
    memory.write(TLB_PAGE_B, &[TLB_PAGE_B_VALUE])?;
    memory.write(SHOOTDOWN_ACK, &[0])?;
    vm.register_guest_memory(memory)?;

    let mut bsp_vcpu = vm.create_vcpu(FIRST_VCPU_ID)?;
    let ap_vcpu = vm.create_vcpu(SECOND_VCPU_ID)?;
    bsp_vcpu.initialize_long_mode(bsp_layout.boot_layout())?;
    let _ = bsp_vcpu.configure_legacy_pic_extint()?;
    let initial_ap_mp_state = ap_vcpu.multiprocessing_state_raw()?;
    if initial_ap_mp_state != KVM_MP_STATE_UNINITIALIZED {
        return Err(verification_error(
            SECOND_VCPU_ID.get(),
            "initial AP MP state",
            format!("expected {KVM_MP_STATE_UNINITIALIZED}, got {initial_ap_mp_state}"),
        ));
    }

    let mut bsp_port_io = PortIoBus::with_debug_port();
    let mut bsp_io_exits = Vec::new();
    for (byte, stage) in [
        (b'0', "pre-INIT"),
        (b'I', "INIT assert"),
        (b'D', "INIT deassert"),
        (b'S', "SIPI"),
    ] {
        bsp_io_exits.push(run_expected_debug_output(
            &mut bsp_vcpu,
            &mut bsp_port_io,
            byte,
            stage,
        )?);
    }

    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (command_tx, command_rx) = mpsc::channel::<ApWorkerCommand>();
    let worker = std::thread::spawn(move || -> Result<ApWorkerResult, Error> {
        let mut ap_vcpu = ap_vcpu;
        let startup = require_init_sipi_startup_state(&mut ap_vcpu)?;
        let mut port_io = PortIoBus::with_debug_port();
        let mut io_exits = Vec::new();
        for (byte, stage) in [
            (b'A', "AP startup"),
            (b'L', "AP long-mode entry"),
            (b'R', "AP alias prime"),
        ] {
            io_exits.push(run_expected_debug_output(
                &mut ap_vcpu,
                &mut port_io,
                byte,
                stage,
            )?);
        }
        let ready_rflags = ap_vcpu.registers()?.rflags;
        require_interrupt_disabled_flags(SECOND_VCPU_ID.get(), "AP TLB ready state", ready_rflags)?;
        let special = ap_vcpu.capture_special_register_snapshot()?;
        let idt = special.idt();
        if idt.base() != LONG_MODE_INTERRUPT_IDT_ADDR.get()
            || idt.limit() != TLB_SHOOTDOWN_IDT_LIMIT
        {
            return Err(verification_error(
                SECOND_VCPU_ID.get(),
                "AP TLB ready IDT",
                format!("unexpected IDT {:#x}/{:#x}", idt.base(), idt.limit()),
            ));
        }
        ready_tx.send(()).map_err(|_| {
            verification_error(
                SECOND_VCPU_ID.get(),
                "AP ready channel",
                "BSP dropped readiness receiver",
            )
        })?;
        match command_rx.recv().map_err(|_| {
            verification_error(
                SECOND_VCPU_ID.get(),
                "AP resume channel",
                "BSP dropped resume sender",
            )
        })? {
            ApWorkerCommand::Continue => {}
            ApWorkerCommand::Abort => {
                return Err(verification_error(
                    SECOND_VCPU_ID.get(),
                    "AP abort",
                    "BSP failed before shootdown IPI",
                ))
            }
        }
        for (byte, stage) in [
            (b'I', "AP shootdown handler"),
            (b'B', "AP post-shootdown alias read"),
            (b'D', "AP completion"),
        ] {
            io_exits.push(run_expected_debug_output(
                &mut ap_vcpu,
                &mut port_io,
                byte,
                stage,
            )?);
        }
        let completion_rflags = ap_vcpu.registers()?.rflags;
        require_interrupt_enabled_flags(
            SECOND_VCPU_ID.get(),
            "AP TLB completion state",
            completion_rflags,
        )?;
        let regs = ap_vcpu.capture_register_snapshot()?;
        let special = ap_vcpu.capture_special_register_snapshot()?;
        let cs = special.cs();
        let ss = special.ss();
        let gdt = special.gdt();
        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
        if proof.as_slice() != AP_TLB_PROOF {
            return Err(verification_error(
                SECOND_VCPU_ID.get(),
                "AP TLB proof",
                format!("expected {AP_TLB_PROOF:?}, got {proof:?}"),
            ));
        }
        Ok(ApWorkerResult {
            io_exits,
            proof,
            ready_rflags,
            completion_rflags,
            startup,
            rsp: regs.rsp(),
            cs_selector: cs.selector(),
            cs_long: cs.l(),
            ss_selector: ss.selector(),
            gdt_base: gdt.base(),
            gdt_limit: gdt.limit(),
            idt_base: special.idt().base(),
            idt_limit: special.idt().limit(),
            cr0: special.cr0(),
            cr3: special.cr3(),
            cr4: special.cr4(),
            efer: special.efer(),
        })
    });
    ready_rx.recv().map_err(|_| {
        verification_error(
            SECOND_VCPU_ID.get(),
            "AP ready receive",
            "AP exited before priming alias",
        )
    })?;
    for (byte, stage) in [(b'P', "shared PTE mutation"), (b'X', "shootdown IPI")] {
        match run_expected_debug_output(&mut bsp_vcpu, &mut bsp_port_io, byte, stage) {
            Ok(exit) => bsp_io_exits.push(exit),
            Err(error) => {
                let _ = command_tx.send(ApWorkerCommand::Abort);
                let _ = worker.join();
                return Err(error);
            }
        }
    }
    command_tx.send(ApWorkerCommand::Continue).map_err(|_| {
        verification_error(
            FIRST_VCPU_ID.get(),
            "AP resume command",
            "AP exited before shootdown IPI resume",
        )
    })?;
    for (byte, stage) in [
        (b'A', "shootdown acknowledgement"),
        (b'D', "BSP completion"),
    ] {
        bsp_io_exits.push(run_expected_debug_output(
            &mut bsp_vcpu,
            &mut bsp_port_io,
            byte,
            stage,
        )?);
    }
    let bsp_proof = bsp_port_io.debug_output().unwrap_or(&[]).to_vec();
    if bsp_proof.as_slice() != BSP_TLB_PROOF {
        return Err(verification_error(
            FIRST_VCPU_ID.get(),
            "BSP TLB proof",
            format!("expected {BSP_TLB_PROOF:?}, got {bsp_proof:?}"),
        ));
    }
    let ap = worker.join().map_err(|_| {
        verification_error(
            SECOND_VCPU_ID.get(),
            "AP TLB worker join",
            "AP worker panicked",
        )
    })??;
    validate_ap_state(&ap)?;
    let guest_memory = vm
        .guest_memory()
        .expect("registered TLB-shootdown memory remains VM-owned");
    let mut pte_bytes = [0_u8; 8];
    guest_memory.read(TLB_TARGET_PTE, &mut pte_bytes)?;
    let final_pte = u64::from_le_bytes(pte_bytes);
    let mut ack = [0_u8; 1];
    let mut page_a = [0_u8; 1];
    let mut page_b = [0_u8; 1];
    guest_memory.read(SHOOTDOWN_ACK, &mut ack)?;
    guest_memory.read(TLB_PAGE_A, &mut page_a)?;
    guest_memory.read(TLB_PAGE_B, &mut page_b)?;
    if final_pte != TLB_FINAL_PTE
        || ack[0] != 0
        || page_a[0] != TLB_PAGE_A_VALUE
        || page_b[0] != TLB_PAGE_B_VALUE
    {
        return Err(verification_error(
            FIRST_VCPU_ID.get(),
            "TLB-shootdown final memory",
            format!(
                "pte={final_pte:#x} ack={} pageA={} pageB={}",
                ack[0], page_a[0], page_b[0]
            ),
        ));
    }
    Ok(TwoVcpuTlbShootdownResult {
        bsp_io_exits,
        ap_io_exits: ap.io_exits,
        bsp_proof,
        ap_proof: ap.proof,
        state: TlbShootdownState {
            initial_ap_mp_state,
            startup_rip: ap.startup.rip,
            startup_cs_selector: ap.startup.cs_selector,
            startup_cs_base: ap.startup.cs_base,
            ready_rflags: ap.ready_rflags,
            completion_rflags: ap.completion_rflags,
            rsp: ap.rsp,
            cs_selector: ap.cs_selector,
            cs_long: ap.cs_long,
            ss_selector: ap.ss_selector,
            gdt_base: ap.gdt_base,
            gdt_limit: ap.gdt_limit,
            idt_base: ap.idt_base,
            idt_limit: ap.idt_limit,
            cr0: ap.cr0,
            cr3: ap.cr3,
            cr4: ap.cr4,
            efer: ap.efer,
        },
        final_pte,
        final_ack: ack[0],
        page_a: page_a[0],
        page_b: page_b[0],
    })
}

fn require_init_sipi_startup_state(vcpu: &mut Vcpu) -> Result<ApStartupObservation, Error> {
    let mp_state = vcpu.accept_init_sipi_startup_handoff()?;
    let registers = vcpu.registers()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let cs = special.cs();
    if mp_state != KVM_MP_STATE_RUNNABLE
        || registers.rip != 0
        || cs.selector() != SIPI_CS_SELECTOR
        || cs.base() != SIPI_CS_BASE
        || special.cr0() & X86_CR0_PROTECTED_MODE_ENABLE != 0
    {
        return Err(verification_error(
            vcpu.id().get(),
            "AP SIPI startup",
            format!(
                "mp={mp_state} rip={:#x} cs={:#x} base={:#x} cr0={:#x}",
                registers.rip,
                cs.selector(),
                cs.base(),
                special.cr0()
            ),
        ));
    }
    Ok(ApStartupObservation {
        mp_state,
        rip: registers.rip,
        cs_selector: cs.selector(),
        cs_base: cs.base(),
    })
}
fn validate_ap_state(ap: &ApWorkerResult) -> Result<(), Error> {
    let valid = ap.startup.mp_state == KVM_MP_STATE_RUNNABLE
        && ap.rsp == AP_LONG_MODE_STACK
        && ap.cs_selector == AP_LONG_MODE_CODE_SELECTOR
        && ap.cs_long == 1
        && ap.ss_selector == AP_LONG_MODE_DATA_SELECTOR
        && ap.gdt_base == AP_LONG_MODE_GDT.get()
        && ap.gdt_limit == AP_LONG_MODE_GDT_LIMIT
        && ap.idt_base == LONG_MODE_INTERRUPT_IDT_ADDR.get()
        && ap.idt_limit == TLB_SHOOTDOWN_IDT_LIMIT
        && ap.cr0 & LONG_MODE_CR0_REQUIRED_BITS == LONG_MODE_CR0_REQUIRED_BITS
        && ap.cr4 & LONG_MODE_CR4_REQUIRED_BITS == LONG_MODE_CR4_REQUIRED_BITS
        && ap.efer & LONG_MODE_EFER_REQUIRED_BITS == LONG_MODE_EFER_REQUIRED_BITS
        && ap.cr3 == LONG_MODE_PML4_ADDR.get();
    if !valid {
        return Err(verification_error(
            SECOND_VCPU_ID.get(),
            "AP post-shootdown long-mode state",
            "AP architectural state drifted from integrated SIPI/long-mode contract",
        ));
    }
    Ok(())
}
fn run_expected_debug_output(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    expected: u8,
    stage: &'static str,
) -> Result<PortIoExit, Error> {
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Io {
        return Err(verification_error(
            vcpu.id().get(),
            stage,
            format!("expected KVM_EXIT_IO, got {exit:?}"),
        ));
    }
    let io = vcpu.port_io_exit()?;
    if io.direction() != PortIoDirection::Out
        || io.port() != DEBUG_PORT
        || io.size() != 1
        || io.count() != 1
        || io.output_data() != [expected]
    {
        return Err(verification_error(
            vcpu.id().get(),
            stage,
            format!("unexpected debug exit {io:?}; expected {expected:#x}"),
        ));
    }
    if port_io.dispatch(&io)? != PortIoService::Output {
        return Err(verification_error(
            vcpu.id().get(),
            stage,
            "debug output requested input response",
        ));
    }
    Ok(io)
}
fn require_interrupt_disabled_flags(
    id: u16,
    stage: &'static str,
    rflags: u64,
) -> Result<(), Error> {
    if rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || rflags & X86_RFLAGS_INTERRUPT_ENABLE != 0
    {
        return Err(verification_error(
            id,
            stage,
            format!("expected bit1 with IF clear, got {rflags:#x}"),
        ));
    }
    Ok(())
}
fn require_interrupt_enabled_flags(id: u16, stage: &'static str, rflags: u64) -> Result<(), Error> {
    if rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
    {
        return Err(verification_error(
            id,
            stage,
            format!("expected bit1+IF, got {rflags:#x}"),
        ));
    }
    Ok(())
}
fn write_u64(memory: &mut GuestMemory, address: GuestPhysAddr, value: u64) -> Result<(), Error> {
    memory.write(address, &value.to_le_bytes())
}
fn verification_error(id: u16, operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id,
        operation,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::long_mode::LONG_MODE_ALIAS_PT_ADDR;

    #[test]
    fn target_alias_uses_shared_page_table_entry_outside_lapic_slot() {
        let pte_index = (TLB_TARGET_VIRTUAL_PAGE - 0x40_0000) / 0x1000;
        assert_eq!(
            TLB_TARGET_PTE.get(),
            LONG_MODE_ALIAS_PT_ADDR.get() + pte_index * 8
        );
        assert_eq!(TLB_TARGET_PTE.get(), 0x4808);
        assert_ne!(TLB_TARGET_VIRTUAL_PAGE, LAPIC_VIRTUAL_PAGE);
        assert_eq!(TLB_PAGE_A.get() | PAGE_TABLE_ENTRY_FLAGS, 0x1_8003);
        assert_eq!(TLB_PAGE_B.get() | PAGE_TABLE_ENTRY_FLAGS, 0x1_9003);
        assert_eq!(
            TLB_PAGE_B.get() | PAGE_TABLE_ENTRY_FLAGS | PAGE_TABLE_ENTRY_ACCESSED,
            TLB_FINAL_PTE
        );
        assert_eq!(TLB_FINAL_PTE, 0x1_9023);
        assert_eq!(TLB_FINAL_PTE & PAGE_TABLE_ENTRY_DIRTY, 0);
    }

    #[test]
    fn ap_preserves_integrated_sipi_transition_and_executes_invlpg_before_ack() {
        const INTEGRATED_PREFIX_LEN: usize = 73;
        assert_eq!(
            &AP_GUEST_BYTES[..INTEGRATED_PREFIX_LEN],
            &[
                0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0, 0xb0, 0x41, 0xe6, 0xe9, 0x0f,
                0x01, 0x16, 0x20, 0x70, 0x0f, 0x20, 0xe0, 0x66, 0x83, 0xc8, 0x20, 0x0f, 0x22, 0xe0,
                0x66, 0xb8, 0x00, 0x10, 0x00, 0x00, 0x0f, 0x22, 0xd8, 0x66, 0xb9, 0x80, 0x00, 0x00,
                0xc0, 0x0f, 0x32, 0x66, 0x0d, 0x00, 0x01, 0x00, 0x00, 0x0f, 0x30, 0x0f, 0x20, 0xc0,
                0x66, 0x0d, 0x01, 0x00, 0x00, 0x80, 0x0f, 0x22, 0xc0, 0x66, 0xea, 0x49, 0x80, 0x00,
                0x00, 0x08, 0x00,
            ]
        );
        assert_eq!(&AP_HANDLER_BYTES[..3], &[0x0f, 0x01, 0x3e]);
        assert!(AP_HANDLER_BYTES
            .windows(4)
            .any(|w| w == [0xb0, b'I', 0xe6, 0xe9]));
    }

    #[test]
    fn proofs_and_idt_contract_are_fixed() {
        assert_eq!(BSP_TLB_PROOF, b"0IDSPXAD");
        assert_eq!(AP_TLB_PROOF, b"ALRIBD");
        assert_eq!(TLB_SHOOTDOWN_VECTOR, 0x54);
        assert_eq!(TLB_SHOOTDOWN_IDT_LIMIT, 0x054f);
        assert_eq!(AP_TLB_IDTR_BYTES[..2], [0x4f, 0x05]);
    }
}
