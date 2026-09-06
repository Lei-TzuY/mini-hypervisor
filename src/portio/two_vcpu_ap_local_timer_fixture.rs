use super::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::error::{Error, HostEnvironmentError};
use crate::interrupt::{LongModeInterruptLayout, LONG_MODE_INTERRUPT_IDT_ADDR};
use crate::kvm::sys::KvmMsiMessage;
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
    SECOND_VCPU_ID, SIPI_VECTOR,
};
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit};
use std::io;
use std::sync::mpsc;
use std::time::Duration;

const RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const BSP_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
const BSP_STACK: u64 = 0x1f_f000;
const AP_TRAMPOLINE: GuestPhysAddr = GuestPhysAddr::new(0x8000);
const SHARED_MARKER: GuestPhysAddr = GuestPhysAddr::new(0x9000);
const SHARED_MARKER_VALUE: u8 = b'K';
const AP_LONG_MODE_GDTR: GuestPhysAddr = GuestPhysAddr::new(0x7020);
const AP_LOCAL_TIMER_IDTR: GuestPhysAddr = GuestPhysAddr::new(0x7040);
const KVM_MP_STATE_RUNNABLE: u32 = 0;
const KVM_MP_STATE_UNINITIALIZED: u32 = 1;
const SIPI_CS_SELECTOR: u16 = 0x0800;
const SIPI_CS_BASE: u64 = 0x8000;
const X86_CR0_PROTECTED_MODE_ENABLE: u64 = 1;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const X86_RFLAGS_INTERRUPT_ENABLE: u64 = 1 << 9;
const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_TIMER_MODE_MASK: u32 = 0x3 << 17;
const APIC_MSI_ADDRESS_BASE: u64 = 0xfee0_0000;
const APIC_MSI_DESTINATION_SHIFT: u32 = 12;
const WATCHDOG_SECONDS: u64 = 5;

pub const AP_LOCAL_TIMER_VECTOR: u8 = 0x53;
pub const AP_LOCAL_TIMER_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x1_3000);
pub const AP_LOCAL_TIMER_IDT_LIMIT: u16 = 0x053f;
pub const AP_LOCAL_TIMER_DIVIDE_CONFIGURATION: u32 = 0x0b;
pub const AP_LOCAL_TIMER_INITIAL_COUNT: u32 = 0x0010_0000;
pub const BSP_TIMER_PROOF: &[u8; 6] = b"0IDSMD";
pub const AP_TIMER_PROOF: &[u8; 7] = b"ALRATWD";

const AP_LONG_MODE_GDT_BYTES: [u8; 24] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x9a, 0xaf, 0x00,
    0xff, 0xff, 0x00, 0x00, 0x00, 0x92, 0xcf, 0x00,
];
const AP_LONG_MODE_GDTR_BYTES: [u8; 6] = [0x17, 0x00, 0x00, 0x70, 0x00, 0x00];
const AP_LOCAL_TIMER_IDTR_BYTES: [u8; 10] =
    [0x3f, 0x05, 0x00, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

// Preserve the integrated BSP INIT/SIPI sequence exactly. Userspace pauses after the SIPI barrier
// while vCPU1 owns its timer lifecycle, then resumes this image only after the AP has stored K.
#[rustfmt::skip]
const BSP_GUEST_BYTES: [u8; 97] = [
    0xfa,
    0x48, 0xbb, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xb0, b'0', 0xe6, 0xe9,
    0xc7, 0x83, 0x10, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x00, 0xc5, 0x00, 0x00,
    0xb0, b'I', 0xe6, 0xe9,
    0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x00, 0x85, 0x00, 0x00,
    0xb0, b'D', 0xe6, 0xe9,
    0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, SIPI_VECTOR, 0x06, 0x00, 0x00,
    0xb0, b'S', 0xe6, 0xe9,
    0x48, 0xb9, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x8a, 0x01,
    0x3c, SHARED_MARKER_VALUE,
    0x75, 0x09,
    0xb0, b'M', 0xe6, 0xe9,
    0xb0, b'D', 0xe6, 0xe9,
    0xf4,
    0xb0, b'F', 0xe6, 0xe9, 0xf4,
];

// Generated locally with GNU as/ld at VMA 0x8000. The first 73 bytes are byte-for-byte identical
// to the integrated AP guest-owned real-mode-to-long-mode transition. In 64-bit mode the AP enables
// its own LAPIC, installs vector 0x53, reports readiness, programs a divide-by-one one-shot timer,
// reports the armed state under CLI, then uses STI;HLT. Only the local timer is expected to enter T.
#[rustfmt::skip]
const AP_TIMER_GUEST_BYTES: [u8; 178] = [
    0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0, 0xb0, 0x41, 0xe6,
    0xe9, 0x0f, 0x01, 0x16, 0x20, 0x70, 0x0f, 0x20, 0xe0, 0x66, 0x83, 0xc8,
    0x20, 0x0f, 0x22, 0xe0, 0x66, 0xb8, 0x00, 0x10, 0x00, 0x00, 0x0f, 0x22,
    0xd8, 0x66, 0xb9, 0x80, 0x00, 0x00, 0xc0, 0x0f, 0x32, 0x66, 0x0d, 0x00,
    0x01, 0x00, 0x00, 0x0f, 0x30, 0x0f, 0x20, 0xc0, 0x66, 0x0d, 0x01, 0x00,
    0x00, 0x80, 0x0f, 0x22, 0xc0, 0x66, 0xea, 0x49, 0x80, 0x00, 0x00, 0x08,
    0x00, 0x66, 0xb8, 0x10, 0x00, 0x8e, 0xd0, 0x8e, 0xd8, 0x8e, 0xc0, 0x48,
    0xc7, 0xc4, 0x00, 0xf0, 0x1e, 0x00, 0xb0, 0x4c, 0xe6, 0xe9, 0x48, 0xc7,
    0xc3, 0x00, 0x00, 0x50, 0x00, 0xc7, 0x83, 0xf0, 0x00, 0x00, 0x00, 0xff,
    0x01, 0x00, 0x00, 0x0f, 0x01, 0x1c, 0x25, 0x40, 0x70, 0x00, 0x00, 0xb0,
    0x52, 0xe6, 0xe9, 0xc7, 0x83, 0xe0, 0x03, 0x00, 0x00, 0x0b, 0x00, 0x00,
    0x00, 0xc7, 0x83, 0x20, 0x03, 0x00, 0x00, 0x53, 0x00, 0x00, 0x00, 0xc7,
    0x83, 0x80, 0x03, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0xb0, 0x41, 0xe6,
    0xe9, 0xfb, 0xf4, 0x48, 0xc7, 0xc1, 0x00, 0x90, 0x00, 0x00, 0xc6, 0x01,
    0x4b, 0xb0, 0x57, 0xe6, 0xe9, 0xb0, 0x44, 0xe6, 0xe9, 0xf4,
];

const AP_TIMER_HANDLER_BYTES: [u8; 16] = [
    0xb0, b'T', 0xe6, 0xe9, 0xc7, 0x83, 0xb0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0xcf,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApLocalTimerState {
    startup_mp_state: u32,
    startup_rip: u64,
    startup_cs_selector: u16,
    startup_cs_base: u64,
    ready_rflags: u64,
    armed_rflags: u64,
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

impl ApLocalTimerState {
    #[must_use]
    pub const fn startup_mp_state(self) -> u32 { self.startup_mp_state }
    #[must_use]
    pub const fn startup_rip(self) -> u64 { self.startup_rip }
    #[must_use]
    pub const fn startup_cs_selector(self) -> u16 { self.startup_cs_selector }
    #[must_use]
    pub const fn startup_cs_base(self) -> u64 { self.startup_cs_base }
    #[must_use]
    pub const fn ready_rflags(self) -> u64 { self.ready_rflags }
    #[must_use]
    pub const fn armed_rflags(self) -> u64 { self.armed_rflags }
    #[must_use]
    pub const fn completion_rflags(self) -> u64 { self.completion_rflags }
    #[must_use]
    pub const fn rsp(self) -> u64 { self.rsp }
    #[must_use]
    pub const fn cs_selector(self) -> u16 { self.cs_selector }
    #[must_use]
    pub const fn cs_long(self) -> u8 { self.cs_long }
    #[must_use]
    pub const fn ss_selector(self) -> u16 { self.ss_selector }
    #[must_use]
    pub const fn gdt_base(self) -> u64 { self.gdt_base }
    #[must_use]
    pub const fn gdt_limit(self) -> u16 { self.gdt_limit }
    #[must_use]
    pub const fn idt_base(self) -> u64 { self.idt_base }
    #[must_use]
    pub const fn idt_limit(self) -> u16 { self.idt_limit }
    #[must_use]
    pub const fn cr0(self) -> u64 { self.cr0 }
    #[must_use]
    pub const fn cr3(self) -> u64 { self.cr3 }
    #[must_use]
    pub const fn cr4(self) -> u64 { self.cr4 }
    #[must_use]
    pub const fn efer(self) -> u64 { self.efer }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuApLocalTimerResult {
    bsp_io_exits: Vec<PortIoExit>,
    ap_io_exits: Vec<PortIoExit>,
    bsp_proof: Vec<u8>,
    ap_proof: Vec<u8>,
    initial_ap_mp_state: u32,
    ap_state: ApLocalTimerState,
    shared_marker: u8,
    watchdog_fired: bool,
}

impl TwoVcpuApLocalTimerResult {
    #[must_use]
    pub fn bsp_io_exits(&self) -> &[PortIoExit] { &self.bsp_io_exits }
    #[must_use]
    pub fn ap_io_exits(&self) -> &[PortIoExit] { &self.ap_io_exits }
    #[must_use]
    pub fn bsp_proof(&self) -> &[u8] { &self.bsp_proof }
    #[must_use]
    pub fn ap_proof(&self) -> &[u8] { &self.ap_proof }
    #[must_use]
    pub const fn initial_ap_mp_state(&self) -> u32 { self.initial_ap_mp_state }
    #[must_use]
    pub const fn ap_state(&self) -> ApLocalTimerState { self.ap_state }
    #[must_use]
    pub const fn shared_marker(&self) -> u8 { self.shared_marker }
    #[must_use]
    pub const fn watchdog_fired(&self) -> bool { self.watchdog_fired }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ApStartupState {
    mp_state: u32,
    rip: u64,
    cs_selector: u16,
    cs_base: u64,
}

#[derive(Debug)]
struct ApWorkerResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    state: ApLocalTimerState,
}

pub fn run_two_vcpu_ap_local_timer() -> Result<TwoVcpuApLocalTimerResult, Error> {
    let bsp_image = FlatGuestImage::new(BSP_ENTRY, BSP_ENTRY, &BSP_GUEST_BYTES)?;
    let ap_image = FlatGuestImage::new(AP_TRAMPOLINE, AP_TRAMPOLINE, &AP_TIMER_GUEST_BYTES)?;
    let handler_image = FlatGuestImage::new(
        AP_LOCAL_TIMER_HANDLER,
        AP_LOCAL_TIMER_HANDLER,
        &AP_TIMER_HANDLER_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    backend.require_mp_state_capability()?;
    backend.require_signal_msi_capability()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(RAM_BASE, LONG_MODE_IDENTITY_MAP_SIZE)?;
    let bsp_layout = LongModeMmioBootLayout::with_device_mappings(
        memory.region(),
        bsp_image.entry(),
        BSP_STACK,
        vec![LongModeMmioPageMapping::new(LAPIC_VIRTUAL_PAGE, LAPIC_GPA)],
    )
    .expect("fixed AP-local-timer BSP LAPIC mapping remains valid");
    let ap_interrupt_layout = LongModeInterruptLayout::new(
        memory.region(),
        AP_TRAMPOLINE,
        AP_LONG_MODE_STACK,
        AP_LOCAL_TIMER_VECTOR,
        AP_LOCAL_TIMER_HANDLER,
    )
    .expect("fixed AP-local-timer interrupt layout remains valid");
    if ap_interrupt_layout.idt_base() != LONG_MODE_INTERRUPT_IDT_ADDR
        || ap_interrupt_layout.idt_limit() != AP_LOCAL_TIMER_IDT_LIMIT
    {
        return Err(verification_error(
            SECOND_VCPU_ID.get(),
            "AP-local-timer IDT layout",
            format!(
                "expected IDT {:#x}/{AP_LOCAL_TIMER_IDT_LIMIT:#x}, got {:#x}/{:#x}",
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
    memory.write(AP_LOCAL_TIMER_IDTR, &AP_LOCAL_TIMER_IDTR_BYTES)?;
    memory.write(SHARED_MARKER, &[0])?;
    vm.register_guest_memory(memory)?;

    let mut bsp_vcpu = vm.create_vcpu(FIRST_VCPU_ID)?;
    let ap_vcpu = vm.create_vcpu(SECOND_VCPU_ID)?;
    bsp_vcpu.initialize_long_mode(bsp_layout.boot_layout())?;
    let _ = bsp_vcpu.configure_legacy_pic_extint()?;
    let initial_ap_mp_state = require_mp_state(
        &ap_vcpu,
        KVM_MP_STATE_UNINITIALIZED,
        "AP-local-timer initial AP MP state",
    )?;

    let mut bsp_port_io = PortIoBus::with_debug_port();
    let mut bsp_io_exits = Vec::new();
    for (byte, stage) in [
        (b'0', "AP-local-timer BSP pre-INIT barrier"),
        (b'I', "AP-local-timer BSP INIT-assert barrier"),
        (b'D', "AP-local-timer BSP INIT-deassert barrier"),
        (b'S', "AP-local-timer BSP SIPI barrier"),
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
        "AP-local-timer BSP post-SIPI state",
        bsp_vcpu.registers()?.rflags,
    )?;

    let (armed_tx, armed_rx) = mpsc::channel::<()>();
    let (completion_tx, completion_rx) = mpsc::channel::<()>();
    let worker = std::thread::spawn(move || -> Result<ApWorkerResult, Error> {
        let mut ap_vcpu = ap_vcpu;
        let mut port_io = PortIoBus::with_debug_port();
        let startup = require_init_sipi_startup_state(&mut ap_vcpu)?;
        let mut io_exits = Vec::new();

        for (byte, stage) in [
            (b'A', "AP-local-timer real-mode startup"),
            (b'L', "AP-local-timer 64-bit entry"),
            (b'R', "AP-local-timer readiness barrier"),
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
            "AP-local-timer readiness state",
            ready_rflags,
        )?;
        let ready_special = ap_vcpu.capture_special_register_snapshot()?;
        let idt = ready_special.idt();
        if idt.base() != LONG_MODE_INTERRUPT_IDT_ADDR.get()
            || idt.limit() != AP_LOCAL_TIMER_IDT_LIMIT
        {
            return Err(verification_error(
                SECOND_VCPU_ID.get(),
                "AP-local-timer readiness IDT",
                format!(
                    "expected IDT {:#x}/{AP_LOCAL_TIMER_IDT_LIMIT:#x}, got {:#x}/{:#x}",
                    LONG_MODE_INTERRUPT_IDT_ADDR.get(), idt.base(), idt.limit()
                ),
            ));
        }

        io_exits.push(run_expected_debug_output(
            &mut ap_vcpu,
            &mut port_io,
            b'A',
            "AP-local-timer armed barrier",
        )?);
        let armed_rflags = ap_vcpu.registers()?.rflags;
        require_interrupt_disabled_flags(
            SECOND_VCPU_ID.get(),
            "AP-local-timer armed state",
            armed_rflags,
        )?;
        armed_tx.send(()).map_err(|_| {
            verification_error(
                SECOND_VCPU_ID.get(),
                "AP-local-timer armed channel",
                "BSP thread dropped armed receiver",
            )
        })?;

        for (byte, stage) in [
            (b'T', "AP-local-timer vector-0x53 handler"),
            (b'W', "AP-local-timer resumed mainline"),
            (b'D', "AP-local-timer completion barrier"),
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
            "AP-local-timer completion state",
            completion_rflags,
        )?;
        let state = capture_ap_state(
            &ap_vcpu,
            startup,
            ready_rflags,
            armed_rflags,
            completion_rflags,
            idt.base(),
            idt.limit(),
        )?;
        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
        if proof.as_slice() != AP_TIMER_PROOF {
            return Err(verification_error(
                SECOND_VCPU_ID.get(),
                "AP-local-timer AP proof",
                format!("expected {AP_TIMER_PROOF:?}, got {proof:?}"),
            ));
        }
        completion_tx.send(()).map_err(|_| {
            verification_error(
                SECOND_VCPU_ID.get(),
                "AP-local-timer completion channel",
                "BSP thread dropped completion receiver",
            )
        })?;
        Ok(ApWorkerResult { io_exits, proof, state })
    });

    if armed_rx.recv().is_err() {
        return worker.join().map_err(|_| {
            verification_error(
                SECOND_VCPU_ID.get(),
                "AP-local-timer worker join before armed state",
                "AP worker panicked before reporting timer armed",
            )
        })?;
    }

    let watchdog_fired = match completion_rx.recv_timeout(Duration::from_secs(WATCHDOG_SECONDS)) {
        Ok(()) => false,
        Err(mpsc::RecvTimeoutError::Disconnected) => false,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let address = APIC_MSI_ADDRESS_BASE
                | ((u64::from(SECOND_VCPU_ID.get())) << APIC_MSI_DESTINATION_SHIFT);
            vm.signal_msi(KvmMsiMessage::new(address, u32::from(AP_LOCAL_TIMER_VECTOR)))?;
            true
        }
    };

    let ap = worker.join().map_err(|_| {
        verification_error(
            SECOND_VCPU_ID.get(),
            "AP-local-timer worker join",
            "AP worker panicked",
        )
    })??;
    if watchdog_fired {
        return Err(verification_error(
            SECOND_VCPU_ID.get(),
            "AP-local-timer watchdog",
            "targeted-MSI watchdog had to wake the AP; local LAPIC timer delivery was not proven",
        ));
    }

    let mut marker = [0_u8; 1];
    vm.guest_memory()
        .expect("registered AP-local-timer guest memory remains VM-owned")
        .read(SHARED_MARKER, &mut marker)?;
    if marker[0] != SHARED_MARKER_VALUE {
        return Err(verification_error(
            SECOND_VCPU_ID.get(),
            "AP-local-timer shared marker",
            format!("expected {SHARED_MARKER_VALUE:#x}, got {:#x}", marker[0]),
        ));
    }

    for (byte, stage) in [
        (b'M', "AP-local-timer BSP marker observation"),
        (b'D', "AP-local-timer BSP completion barrier"),
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
        "AP-local-timer BSP completion state",
        bsp_vcpu.registers()?.rflags,
    )?;
    let bsp_proof = bsp_port_io.debug_output().unwrap_or(&[]).to_vec();
    if bsp_proof.as_slice() != BSP_TIMER_PROOF {
        return Err(verification_error(
            FIRST_VCPU_ID.get(),
            "AP-local-timer BSP proof",
            format!("expected {BSP_TIMER_PROOF:?}, got {bsp_proof:?}"),
        ));
    }

    Ok(TwoVcpuApLocalTimerResult {
        bsp_io_exits,
        ap_io_exits: ap.io_exits,
        bsp_proof,
        ap_proof: ap.proof,
        initial_ap_mp_state,
        ap_state: ap.state,
        shared_marker: marker[0],
        watchdog_fired,
    })
}

fn require_init_sipi_startup_state(vcpu: &mut Vcpu) -> Result<ApStartupState, Error> {
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
            "AP-local-timer AP startup state",
            format!(
                "expected MP={KVM_MP_STATE_RUNNABLE}, RIP=0, CS={SIPI_CS_SELECTOR:#x} base={SIPI_CS_BASE:#x}, CR0.PE=0; got MP={mp_state}, RIP={:#x}, CS={:#x} base={:#x}, CR0={:#x}",
                registers.rip, cs.selector(), cs.base(), special.cr0()
            ),
        ));
    }
    Ok(ApStartupState {
        mp_state,
        rip: registers.rip,
        cs_selector: cs.selector(),
        cs_base: cs.base(),
    })
}

fn capture_ap_state(
    vcpu: &Vcpu,
    startup: ApStartupState,
    ready_rflags: u64,
    armed_rflags: u64,
    completion_rflags: u64,
    idt_base: u64,
    idt_limit: u16,
) -> Result<ApLocalTimerState, Error> {
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
            "AP-local-timer guest-owned long-mode state",
            "AP did not retain the integrated guest-owned long-mode architectural contract",
        ));
    }
    Ok(ApLocalTimerState {
        startup_mp_state: startup.mp_state,
        startup_rip: startup.rip,
        startup_cs_selector: startup.cs_selector,
        startup_cs_base: startup.cs_base,
        ready_rflags,
        armed_rflags,
        completion_rflags,
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

fn require_interrupt_enabled_flags(id: u16, stage: &'static str, rflags: u64) -> Result<(), Error> {
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
    fn timer_machine_code_preserves_startup_and_programs_one_shot_lapic() {
        assert_eq!(AP_TIMER_GUEST_BYTES.len(), 178);
        assert_eq!(&AP_TIMER_GUEST_BYTES[..73], &[
            0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0, 0xb0, 0x41, 0xe6,
            0xe9, 0x0f, 0x01, 0x16, 0x20, 0x70, 0x0f, 0x20, 0xe0, 0x66, 0x83, 0xc8,
            0x20, 0x0f, 0x22, 0xe0, 0x66, 0xb8, 0x00, 0x10, 0x00, 0x00, 0x0f, 0x22,
            0xd8, 0x66, 0xb9, 0x80, 0x00, 0x00, 0xc0, 0x0f, 0x32, 0x66, 0x0d, 0x00,
            0x01, 0x00, 0x00, 0x0f, 0x30, 0x0f, 0x20, 0xc0, 0x66, 0x0d, 0x01, 0x00,
            0x00, 0x80, 0x0f, 0x22, 0xc0, 0x66, 0xea, 0x49, 0x80, 0x00, 0x00, 0x08,
            0x00,
        ]);
        assert!(AP_TIMER_GUEST_BYTES.windows(10).any(|window| window == [
            0xc7, 0x83, 0xe0, 0x03, 0, 0,
            AP_LOCAL_TIMER_DIVIDE_CONFIGURATION as u8, 0, 0, 0,
        ]));
        assert!(AP_TIMER_GUEST_BYTES.windows(10).any(|window| window == [
            0xc7, 0x83, 0x20, 0x03, 0, 0, AP_LOCAL_TIMER_VECTOR, 0, 0, 0,
        ]));
        assert!(AP_TIMER_GUEST_BYTES.windows(10).any(|window| window == [
            0xc7, 0x83, 0x80, 0x03, 0, 0, 0, 0, 0x10, 0,
        ]));
        assert!(AP_TIMER_GUEST_BYTES.windows(2).any(|window| window == [0xfb, 0xf4]));
        assert_eq!(AP_TIMER_HANDLER_BYTES[..4], [0xb0, b'T', 0xe6, 0xe9]);
        assert_eq!(AP_LOCAL_TIMER_VECTOR, 0x53);
        assert_eq!(AP_LOCAL_TIMER_IDT_LIMIT, 0x53f);
        assert_eq!(AP_LOCAL_TIMER_DIVIDE_CONFIGURATION & 0x0b, 0x0b);
        assert_ne!(AP_LOCAL_TIMER_INITIAL_COUNT, 0);
        assert_eq!(AP_LOCAL_TIMER_VECTOR as u32 & APIC_LVT_MASKED, 0);
        assert_eq!(AP_LOCAL_TIMER_VECTOR as u32 & APIC_LVT_TIMER_MODE_MASK, 0);
        assert_eq!(0x1ff & APIC_SPIV_SOFTWARE_ENABLE, APIC_SPIV_SOFTWARE_ENABLE);
    }

    #[test]
    fn fixed_proofs_keep_bsp_startup_separate_from_ap_timer_ownership() {
        assert_eq!(BSP_TIMER_PROOF, b"0IDSMD");
        assert_eq!(AP_TIMER_PROOF, b"ALRATWD");
        assert_eq!(SHARED_MARKER.get(), 0x9000);
        assert_eq!(SHARED_MARKER_VALUE, b'K');
    }
}
