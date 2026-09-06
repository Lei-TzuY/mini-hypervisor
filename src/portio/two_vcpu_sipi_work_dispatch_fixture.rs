use super::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::error::{Error, HostEnvironmentError};
use crate::execution::run_vcpu_until_stopped;
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
    AP_LONG_MODE_CODE_SELECTOR, AP_LONG_MODE_DATA_SELECTOR, AP_LONG_MODE_GDT, AP_LONG_MODE_GDTR,
    AP_LONG_MODE_GDT_LIMIT, AP_LONG_MODE_IPI_HANDLER, AP_LONG_MODE_IPI_IDTR,
    AP_LONG_MODE_IPI_IDT_LIMIT, AP_LONG_MODE_IPI_VECTOR, AP_LONG_MODE_STACK, FIRST_VCPU_ID,
    LAPIC_GPA, LAPIC_VIRTUAL_PAGE, SECOND_VCPU_ID,
};
#[cfg(test)]
use crate::portio::two_vcpu_init_sipi_fixture::{
    ICR_HIGH_VALUE, INIT_ASSERT_VALUE, INIT_DEASSERT_VALUE, SIPI_VALUE,
};
use crate::portio::two_vcpu_work_dispatch_fixture::{
    WORK_ACK_OFFSET, WORK_COMMAND_OFFSET, WORK_MAILBOX, WORK_PAYLOAD, WORK_RESULT,
    WORK_RESULT_OFFSET,
};
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit};
use crate::vmexit::VmExitReport;
use std::io;
use std::sync::mpsc;

const RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const BSP_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
const BSP_STACK: u64 = 0x1f_f000;
const AP_TRAMPOLINE: GuestPhysAddr = GuestPhysAddr::new(0x8000);
const MAILBOX_EXTENT: usize = 0x19;
const KVM_MP_STATE_RUNNABLE: u32 = 0;
const KVM_MP_STATE_UNINITIALIZED: u32 = 1;
const SIPI_CS_SELECTOR: u16 = 0x0800;
const SIPI_CS_BASE: u64 = 0x8000;
const X86_CR0_PROTECTED_MODE_ENABLE: u64 = 1;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const X86_RFLAGS_INTERRUPT_ENABLE: u64 = 1 << 9;

pub const BSP_COMPOSED_PROOF: &[u8; 8] = b"0IDSCXVD";
pub const AP_COMPOSED_PROOF: &[u8; 6] = b"ALRIPD";
pub const BSP_TERMINAL_RIP: u64 = 0x1_009b;
pub const AP_TERMINAL_RIP: u64 = 0x80a6;

const AP_LONG_MODE_GDT_BYTES: [u8; 24] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x9a, 0xaf, 0x00,
    0xff, 0xff, 0x00, 0x00, 0x00, 0x92, 0xcf, 0x00,
];
const AP_LONG_MODE_GDTR_BYTES: [u8; 6] = [0x17, 0x00, 0x00, 0x70, 0x00, 0x00];
const AP_LONG_MODE_IPI_IDTR_BYTES: [u8; 10] =
    [0x2f, 0x05, 0x00, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

// Generated locally with GNU as from 64-bit code at VMA 0x10000. The BSP owns AP startup,
// publishes payload before command through an implicitly locked XCHG, then sends the already
// integrated fixed xAPIC vector 0x52. A second locked XCHG consumes the AP acknowledgement before
// result validation. Bounded polling fails closed with F instead of spinning indefinitely.
#[rustfmt::skip]
const BSP_GUEST_BYTES: [u8; 160] = [
    0xfa, 0x48, 0xc7, 0xc3, 0x00, 0x00, 0x50, 0x00, 0xb0, 0x30, 0xe6, 0xe9,
    0xc7, 0x83, 0x10, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0xc7, 0x83,
    0x00, 0x03, 0x00, 0x00, 0x00, 0xc5, 0x00, 0x00, 0xb0, 0x49, 0xe6, 0xe9,
    0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x00, 0x85, 0x00, 0x00, 0xb0, 0x44,
    0xe6, 0xe9, 0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x08, 0x06, 0x00, 0x00,
    0xb0, 0x53, 0xe6, 0xe9, 0x48, 0xc7, 0xc1, 0x00, 0x90, 0x00, 0x00, 0xc6,
    0x01, 0x21, 0xb0, 0x01, 0x86, 0x41, 0x08, 0x84, 0xc0, 0x75, 0x48, 0xb0,
    0x43, 0xe6, 0xe9, 0xc7, 0x83, 0x10, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x52, 0x00, 0x00, 0x00, 0xb0,
    0x58, 0xe6, 0xe9, 0xba, 0x00, 0x00, 0x00, 0x08, 0x80, 0x79, 0x18, 0x01,
    0x74, 0x08, 0xf3, 0x90, 0xff, 0xca, 0x75, 0xf4, 0xeb, 0x19, 0x31, 0xc0,
    0x86, 0x41, 0x18, 0x3c, 0x01, 0x75, 0x10, 0x8a, 0x41, 0x10, 0x3c, 0x42,
    0x75, 0x09, 0xb0, 0x56, 0xe6, 0xe9, 0xb0, 0x44, 0xe6, 0xe9, 0xf4, 0xb0,
    0x46, 0xe6, 0xe9, 0xf4,
];

// The first 73 bytes preserve the integrated SIPI real-mode-to-long-mode transition exactly. The
// 64-bit suffix software-enables the AP LAPIC, loads the vector-0x52 IDT, reports readiness with IF
// clear, then uses STI;HLT for the guest-originated work-notification IPI. After IRETQ the AP claims
// the command with locked XCHG, computes 0x21*2, and publishes the acknowledgement with locked XCHG.
#[rustfmt::skip]
const AP_GUEST_BYTES: [u8; 171] = [
    0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0, 0xb0, 0x41, 0xe6,
    0xe9, 0x0f, 0x01, 0x16, 0x20, 0x70, 0x0f, 0x20, 0xe0, 0x66, 0x83, 0xc8,
    0x20, 0x0f, 0x22, 0xe0, 0x66, 0xb8, 0x00, 0x10, 0x00, 0x00, 0x0f, 0x22,
    0xd8, 0x66, 0xb9, 0x80, 0x00, 0x00, 0xc0, 0x0f, 0x32, 0x66, 0x0d, 0x00,
    0x01, 0x00, 0x00, 0x0f, 0x30, 0x0f, 0x20, 0xc0, 0x66, 0x0d, 0x01, 0x00,
    0x00, 0x80, 0x0f, 0x22, 0xc0, 0x66, 0xea, 0x49, 0x80, 0x00, 0x00, 0x08,
    0x00, 0x66, 0xb8, 0x10, 0x00, 0x8e, 0xd0, 0x8e, 0xd8, 0x8e, 0xc0, 0x48,
    0xc7, 0xc4, 0x00, 0xf0, 0x1e, 0x00, 0xb0, 0x4c, 0xe6, 0xe9, 0x48, 0xc7,
    0xc3, 0x00, 0x00, 0x50, 0x00, 0xc7, 0x83, 0xf0, 0x00, 0x00, 0x00, 0xff,
    0x01, 0x00, 0x00, 0x0f, 0x01, 0x1c, 0x25, 0x40, 0x70, 0x00, 0x00, 0x48,
    0xc7, 0xc1, 0x00, 0x90, 0x00, 0x00, 0xb0, 0x52, 0xe6, 0xe9, 0xfb, 0xf4,
    0x31, 0xc0, 0x86, 0x41, 0x08, 0x3c, 0x01, 0x75, 0x19, 0x8a, 0x01, 0x00,
    0xc0, 0x88, 0x41, 0x10, 0xb0, 0x01, 0x86, 0x41, 0x18, 0x84, 0xc0, 0x75,
    0x09, 0xb0, 0x50, 0xe6, 0xe9, 0xb0, 0x44, 0xe6, 0xe9, 0xf4, 0xb0, 0x46,
    0xe6, 0xe9, 0xf4,
];

const AP_HANDLER_BYTES: [u8; 16] = [
    0xb0, b'I', 0xe6, 0xe9, 0xc7, 0x83, 0xb0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xcf,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposedMailboxSnapshot {
    payload: u8,
    command: u8,
    result: u8,
    ack: u8,
}

impl ComposedMailboxSnapshot {
    #[must_use]
    pub const fn payload(self) -> u8 {
        self.payload
    }

    #[must_use]
    pub const fn command(self) -> u8 {
        self.command
    }

    #[must_use]
    pub const fn result(self) -> u8 {
        self.result
    }

    #[must_use]
    pub const fn ack(self) -> u8 {
        self.ack
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposedApState {
    ready_rflags: u64,
    completion_rflags: u64,
    startup_mp_state: u32,
    startup_rip: u64,
    startup_cs_selector: u16,
    startup_cs_base: u64,
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

impl ComposedApState {
    #[must_use]
    pub const fn ready_rflags(self) -> u64 {
        self.ready_rflags
    }

    #[must_use]
    pub const fn completion_rflags(self) -> u64 {
        self.completion_rflags
    }

    #[must_use]
    pub const fn startup_mp_state(self) -> u32 {
        self.startup_mp_state
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
pub struct SipiIpiWorkDispatchResult {
    bsp_io_exits: Vec<PortIoExit>,
    ap_io_exits: Vec<PortIoExit>,
    bsp_proof: Vec<u8>,
    ap_proof: Vec<u8>,
    mailbox: ComposedMailboxSnapshot,
    initial_ap_mp_state: u32,
    ap_state: ComposedApState,
    bsp_report: VmExitReport,
    ap_report: VmExitReport,
}

impl SipiIpiWorkDispatchResult {
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
    pub const fn mailbox(&self) -> ComposedMailboxSnapshot {
        self.mailbox
    }

    #[must_use]
    pub const fn initial_ap_mp_state(&self) -> u32 {
        self.initial_ap_mp_state
    }

    #[must_use]
    pub const fn ap_state(&self) -> ComposedApState {
        self.ap_state
    }

    #[must_use]
    pub const fn bsp_report(&self) -> VmExitReport {
        self.bsp_report
    }

    #[must_use]
    pub const fn ap_report(&self) -> VmExitReport {
        self.ap_report
    }
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
    state: ComposedApState,
    report: VmExitReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ApStartupObservation {
    mp_state: u32,
    rip: u64,
    cs_selector: u16,
    cs_base: u64,
}

pub fn run_sipi_ipi_work_dispatch() -> Result<SipiIpiWorkDispatchResult, Error> {
    let bsp_image = FlatGuestImage::new(BSP_ENTRY, BSP_ENTRY, &BSP_GUEST_BYTES)?;
    let ap_image = FlatGuestImage::new(AP_TRAMPOLINE, AP_TRAMPOLINE, &AP_GUEST_BYTES)?;
    let handler_image = FlatGuestImage::new(
        AP_LONG_MODE_IPI_HANDLER,
        AP_LONG_MODE_IPI_HANDLER,
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
    .expect("fixed composed BSP LAPIC mapping remains valid");
    let ap_interrupt_layout = LongModeInterruptLayout::new(
        memory.region(),
        AP_TRAMPOLINE,
        AP_LONG_MODE_STACK,
        AP_LONG_MODE_IPI_VECTOR,
        AP_LONG_MODE_IPI_HANDLER,
    )
    .expect("fixed composed AP interrupt-table layout remains valid");
    if ap_interrupt_layout.idt_base() != LONG_MODE_INTERRUPT_IDT_ADDR
        || ap_interrupt_layout.idt_limit() != AP_LONG_MODE_IPI_IDT_LIMIT
    {
        return Err(verification_error(
            SECOND_VCPU_ID.get(),
            "composed AP interrupt-table layout",
            format!(
                "expected IDT {:#x}/{AP_LONG_MODE_IPI_IDT_LIMIT:#x}, got {:#x}/{:#x}",
                LONG_MODE_INTERRUPT_IDT_ADDR.get(),
                ap_interrupt_layout.idt_base().get(),
                ap_interrupt_layout.idt_limit()
            ),
        ));
    }
    ap_interrupt_layout.install_tables(&mut memory)?;
    bsp_layout.install_page_tables(&mut memory)?;
    bsp_image.load(&mut memory)?;
    ap_image.load(&mut memory)?;
    handler_image.load(&mut memory)?;
    memory.write(AP_LONG_MODE_GDT, &AP_LONG_MODE_GDT_BYTES)?;
    memory.write(AP_LONG_MODE_GDTR, &AP_LONG_MODE_GDTR_BYTES)?;
    memory.write(AP_LONG_MODE_IPI_IDTR, &AP_LONG_MODE_IPI_IDTR_BYTES)?;
    memory.write(WORK_MAILBOX, &[0_u8; MAILBOX_EXTENT])?;
    vm.register_guest_memory(memory)?;

    let mut bsp_vcpu = vm.create_vcpu(FIRST_VCPU_ID)?;
    let ap_vcpu = vm.create_vcpu(SECOND_VCPU_ID)?;
    bsp_vcpu.initialize_long_mode(bsp_layout.boot_layout())?;
    let _ = bsp_vcpu.configure_legacy_pic_extint()?;
    let initial_ap_mp_state = require_mp_state(
        &ap_vcpu,
        KVM_MP_STATE_UNINITIALIZED,
        "composed initial AP MP state",
    )?;

    let mut bsp_port_io = PortIoBus::with_debug_port();
    let mut bsp_io_exits = Vec::new();
    for (byte, stage) in [
        (b'0', "composed BSP pre-INIT barrier"),
        (b'I', "composed BSP INIT-assert barrier"),
        (b'D', "composed BSP INIT-deassert barrier"),
        (b'S', "composed BSP SIPI barrier"),
    ] {
        bsp_io_exits.push(run_expected_debug_output(
            &mut bsp_vcpu,
            &mut bsp_port_io,
            byte,
            stage,
        )?);
    }
    require_interrupt_disabled_flags(
        FIRST_VCPU_ID.get(),
        "composed BSP post-SIPI state",
        bsp_vcpu.registers()?.rflags,
    )?;

    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (command_tx, command_rx) = mpsc::channel::<ApWorkerCommand>();
    let worker = std::thread::spawn(move || -> Result<ApWorkerResult, Error> {
        let mut ap_vcpu = ap_vcpu;
        let mut port_io = PortIoBus::with_debug_port();
        let startup = require_init_sipi_startup_state(&mut ap_vcpu)?;
        let mut io_exits = Vec::new();
        for (byte, stage) in [
            (b'A', "composed AP real-mode startup"),
            (b'L', "composed AP 64-bit entry"),
            (b'R', "composed AP work-IPI readiness"),
        ] {
            io_exits.push(run_expected_debug_output(
                &mut ap_vcpu,
                &mut port_io,
                byte,
                stage,
            )?);
        }
        let ready_rflags = ap_vcpu.registers()?.rflags;
        require_interrupt_disabled_flags(
            SECOND_VCPU_ID.get(),
            "composed AP readiness state",
            ready_rflags,
        )?;
        let ready_special = ap_vcpu.capture_special_register_snapshot()?;
        let ready_idt = ready_special.idt();
        if ready_idt.base() != LONG_MODE_INTERRUPT_IDT_ADDR.get()
            || ready_idt.limit() != AP_LONG_MODE_IPI_IDT_LIMIT
        {
            return Err(verification_error(
                SECOND_VCPU_ID.get(),
                "composed AP readiness IDT",
                format!(
                    "expected IDT {:#x}/{AP_LONG_MODE_IPI_IDT_LIMIT:#x}, got {:#x}/{:#x}",
                    LONG_MODE_INTERRUPT_IDT_ADDR.get(),
                    ready_idt.base(),
                    ready_idt.limit()
                ),
            ));
        }
        ready_tx.send(()).map_err(|_| {
            verification_error(
                SECOND_VCPU_ID.get(),
                "composed AP readiness channel",
                "BSP thread dropped readiness receiver",
            )
        })?;
        match command_rx.recv().map_err(|_| {
            verification_error(
                SECOND_VCPU_ID.get(),
                "composed AP resume channel",
                "BSP thread dropped resume sender",
            )
        })? {
            ApWorkerCommand::Continue => {}
            ApWorkerCommand::Abort => {
                return Err(verification_error(
                    SECOND_VCPU_ID.get(),
                    "composed AP abort",
                    "BSP failed before the work-notification IPI completed",
                ));
            }
        }
        for (byte, stage) in [
            (b'I', "composed AP work-notification IPI handler"),
            (b'P', "composed AP result/ack publication"),
            (b'D', "composed AP completion barrier"),
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
            "composed AP completion state",
            completion_rflags,
        )?;
        let terminal = run_vcpu_until_stopped(&mut ap_vcpu, &mut port_io, 1)?;
        let state = capture_composed_ap_state(
            &ap_vcpu,
            startup,
            ready_rflags,
            completion_rflags,
            ready_idt.base(),
            ready_idt.limit(),
        )?;
        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
        if proof.as_slice() != AP_COMPOSED_PROOF {
            return Err(verification_error(
                SECOND_VCPU_ID.get(),
                "composed AP proof",
                format!("expected {AP_COMPOSED_PROOF:?}, got {proof:?}"),
            ));
        }
        Ok(ApWorkerResult {
            io_exits,
            proof,
            state,
            report: terminal.report(),
        })
    });

    ready_rx.recv().map_err(|_| {
        verification_error(
            SECOND_VCPU_ID.get(),
            "composed AP readiness receive",
            "AP worker exited before reporting long-mode readiness",
        )
    })?;

    let bsp_command = match run_expected_debug_output(
        &mut bsp_vcpu,
        &mut bsp_port_io,
        b'C',
        "composed BSP command publication",
    ) {
        Ok(exit) => exit,
        Err(error) => {
            let _ = command_tx.send(ApWorkerCommand::Abort);
            let _ = worker.join();
            return Err(error);
        }
    };
    bsp_io_exits.push(bsp_command);
    let bsp_ipi = match run_expected_debug_output(
        &mut bsp_vcpu,
        &mut bsp_port_io,
        b'X',
        "composed BSP work-notification IPI",
    ) {
        Ok(exit) => exit,
        Err(error) => {
            let _ = command_tx.send(ApWorkerCommand::Abort);
            let _ = worker.join();
            return Err(error);
        }
    };
    bsp_io_exits.push(bsp_ipi);
    command_tx.send(ApWorkerCommand::Continue).map_err(|_| {
        verification_error(
            FIRST_VCPU_ID.get(),
            "composed AP resume command",
            "AP worker exited before IPI resume",
        )
    })?;

    let bsp_execution = (|| -> Result<(Vec<PortIoExit>, VmExitReport), Error> {
        let mut exits = Vec::new();
        for (byte, stage) in [
            (b'V', "composed BSP result validation"),
            (b'D', "composed BSP completion barrier"),
        ] {
            exits.push(run_expected_debug_output(
                &mut bsp_vcpu,
                &mut bsp_port_io,
                byte,
                stage,
            )?);
        }
        let terminal = run_vcpu_until_stopped(&mut bsp_vcpu, &mut bsp_port_io, 1)?;
        Ok((exits, terminal.report()))
    })();

    let ap = worker.join().map_err(|_| {
        verification_error(
            SECOND_VCPU_ID.get(),
            "composed AP worker join",
            "AP worker panicked",
        )
    })??;
    let (tail_exits, bsp_report) = bsp_execution?;
    bsp_io_exits.extend(tail_exits);

    let bsp_proof = bsp_port_io.debug_output().unwrap_or(&[]).to_vec();
    if bsp_proof.as_slice() != BSP_COMPOSED_PROOF {
        return Err(verification_error(
            FIRST_VCPU_ID.get(),
            "composed BSP proof",
            format!("expected {BSP_COMPOSED_PROOF:?}, got {bsp_proof:?}"),
        ));
    }

    let mut mailbox_bytes = [0_u8; MAILBOX_EXTENT];
    vm.guest_memory()
        .expect("registered composed guest memory remains VM-owned")
        .read(WORK_MAILBOX, &mut mailbox_bytes)?;
    let mailbox = ComposedMailboxSnapshot {
        payload: mailbox_bytes[0],
        command: mailbox_bytes[WORK_COMMAND_OFFSET],
        result: mailbox_bytes[WORK_RESULT_OFFSET],
        ack: mailbox_bytes[WORK_ACK_OFFSET],
    };
    let expected_mailbox = ComposedMailboxSnapshot {
        payload: WORK_PAYLOAD,
        command: 0,
        result: WORK_RESULT,
        ack: 0,
    };
    if mailbox != expected_mailbox {
        return Err(verification_error(
            FIRST_VCPU_ID.get(),
            "composed mailbox completion",
            format!("expected {expected_mailbox:?}, got {mailbox:?}"),
        ));
    }

    Ok(SipiIpiWorkDispatchResult {
        bsp_io_exits,
        ap_io_exits: ap.io_exits,
        bsp_proof,
        ap_proof: ap.proof,
        mailbox,
        initial_ap_mp_state,
        ap_state: ap.state,
        bsp_report,
        ap_report: ap.report,
    })
}

fn require_init_sipi_startup_state(vcpu: &mut Vcpu) -> Result<ApStartupObservation, Error> {
    let mp_state = vcpu.accept_init_sipi_startup_handoff()?;
    let registers = vcpu.registers()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let cs = special.cs();
    if registers.rip != 0
        || cs.selector() != SIPI_CS_SELECTOR
        || cs.base() != SIPI_CS_BASE
        || special.cr0() & X86_CR0_PROTECTED_MODE_ENABLE != 0
    {
        return Err(verification_error(
            vcpu.id().get(),
            "composed AP SIPI startup state",
            format!(
                "expected MP={KVM_MP_STATE_RUNNABLE}, RIP=0, CS={SIPI_CS_SELECTOR:#x} base={SIPI_CS_BASE:#x}, CR0.PE=0; got MP={mp_state}, RIP={:#x}, CS={:#x} base={:#x}, CR0={:#x}",
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

fn capture_composed_ap_state(
    vcpu: &Vcpu,
    startup: ApStartupObservation,
    ready_rflags: u64,
    completion_rflags: u64,
    idt_base: u64,
    idt_limit: u16,
) -> Result<ComposedApState, Error> {
    let registers = vcpu.capture_register_snapshot()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let cs = special.cs();
    let ss = special.ss();
    let gdt = special.gdt();
    let valid = registers.rsp() == AP_LONG_MODE_STACK
        && cs.selector() == AP_LONG_MODE_CODE_SELECTOR
        && cs.l() == 1
        && cs.db() == 0
        && cs.present() == 1
        && ss.selector() == AP_LONG_MODE_DATA_SELECTOR
        && gdt.base() == AP_LONG_MODE_GDT.get()
        && gdt.limit() == AP_LONG_MODE_GDT_LIMIT
        && special.cr0() & LONG_MODE_CR0_REQUIRED_BITS == LONG_MODE_CR0_REQUIRED_BITS
        && special.cr4() & LONG_MODE_CR4_REQUIRED_BITS == LONG_MODE_CR4_REQUIRED_BITS
        && special.efer() & LONG_MODE_EFER_REQUIRED_BITS == LONG_MODE_EFER_REQUIRED_BITS
        && special.cr3() == LONG_MODE_PML4_ADDR.get();
    if !valid {
        return Err(verification_error(
            vcpu.id().get(),
            "composed AP guest-owned long-mode state",
            "AP did not retain the integrated guest-owned long-mode architectural contract",
        ));
    }
    Ok(ComposedApState {
        ready_rflags,
        completion_rflags,
        startup_mp_state: startup.mp_state,
        startup_rip: startup.rip,
        startup_cs_selector: startup.cs_selector,
        startup_cs_base: startup.cs_base,
        rsp: registers.rsp(),
        cs_selector: cs.selector(),
        cs_long: cs.l(),
        ss_selector: ss.selector(),
        gdt_base: gdt.base(),
        gdt_limit: gdt.limit(),
        idt_base,
        idt_limit,
        cr0: special.cr0(),
        cr3: special.cr3(),
        cr4: special.cr4(),
        efer: special.efer(),
    })
}

fn require_mp_state(vcpu: &Vcpu, expected: u32, stage: &'static str) -> Result<u32, Error> {
    let observed = vcpu.multiprocessing_state_raw()?;
    if observed != expected {
        return Err(verification_error(
            vcpu.id().get(),
            stage,
            format!("expected KVM MP state {expected}, got {observed}"),
        ));
    }
    Ok(observed)
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
    let io_exit = vcpu.port_io_exit()?;
    if io_exit.direction() != PortIoDirection::Out
        || io_exit.port() != DEBUG_PORT
        || io_exit.size() != 1
        || io_exit.count() != 1
        || io_exit.output_data() != [expected]
    {
        return Err(verification_error(
            vcpu.id().get(),
            stage,
            format!("unexpected debug output exit: {io_exit:?}; expected byte {expected:#x}"),
        ));
    }
    if port_io.dispatch(&io_exit)? != PortIoService::Output {
        return Err(verification_error(
            vcpu.id().get(),
            stage,
            "debug output unexpectedly requested an input response",
        ));
    }
    Ok(io_exit)
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
            format!("expected architectural bit1 set and IF clear, got RFLAGS {rflags:#x}"),
        ));
    }
    Ok(())
}

fn require_interrupt_enabled_flags(
    id: u16,
    stage: &'static str,
    rflags: u64,
) -> Result<(), Error> {
    if rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
    {
        return Err(verification_error(
            id,
            stage,
            format!("expected architectural bit1 and IF set, got RFLAGS {rflags:#x}"),
        ));
    }
    Ok(())
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

    #[test]
    fn composed_guest_bytes_preserve_sipi_ipi_and_locked_mailbox_contracts() {
        assert_eq!(BSP_GUEST_BYTES.len(), 160);
        assert_eq!(AP_GUEST_BYTES.len(), 171);
        assert_eq!(BSP_COMPOSED_PROOF, b"0IDSCXVD");
        assert_eq!(AP_COMPOSED_PROOF, b"ALRIPD");
        assert_eq!(BSP_TERMINAL_RIP, BSP_ENTRY.get() + 0x9b);
        assert_eq!(AP_TERMINAL_RIP, AP_TRAMPOLINE.get() + 0xa6);
        assert!(BSP_GUEST_BYTES
            .windows(3)
            .any(|window| window == [0x86, 0x41, WORK_COMMAND_OFFSET as u8]));
        assert!(BSP_GUEST_BYTES
            .windows(3)
            .any(|window| window == [0x86, 0x41, WORK_ACK_OFFSET as u8]));
        assert!(AP_GUEST_BYTES
            .windows(3)
            .any(|window| window == [0x86, 0x41, WORK_COMMAND_OFFSET as u8]));
        assert!(AP_GUEST_BYTES
            .windows(3)
            .any(|window| window == [0x86, 0x41, WORK_ACK_OFFSET as u8]));
        assert!(AP_GUEST_BYTES
            .windows(2)
            .any(|window| window == [0xfb, 0xf4]));
        assert!(BSP_GUEST_BYTES.windows(10).any(|window| {
            window
                == [
                    0xc7,
                    0x83,
                    0x00,
                    0x03,
                    0x00,
                    0x00,
                    AP_LONG_MODE_IPI_VECTOR,
                    0,
                    0,
                    0,
                ]
        }));
        assert_eq!(&AP_HANDLER_BYTES[..4], &[0xb0, b'I', 0xe6, 0xe9]);
    }

    #[test]
    fn composed_tables_match_integrated_ap_startup_contract() {
        assert_eq!(
            AP_LONG_MODE_GDT_BYTES[8..16],
            [0xff, 0xff, 0, 0, 0, 0x9a, 0xaf, 0]
        );
        assert_eq!(AP_LONG_MODE_GDTR_BYTES, [0x17, 0, 0, 0x70, 0, 0]);
        assert_eq!(
            AP_LONG_MODE_IPI_IDTR_BYTES,
            [0x2f, 0x05, 0, 0x60, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(AP_LONG_MODE_IPI_VECTOR, 0x52);
        assert_eq!(AP_LONG_MODE_IPI_IDT_LIMIT, 0x52f);
        assert_eq!(ICR_HIGH_VALUE, 0x0100_0000);
        assert_eq!(INIT_ASSERT_VALUE, 0x0000_c500);
        assert_eq!(INIT_DEASSERT_VALUE, 0x0000_8500);
        assert_eq!(SIPI_VALUE, 0x0000_0608);
    }
}
