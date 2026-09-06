use super::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::error::{Error, HostEnvironmentError};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{
    LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS,
    LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PML4_ADDR,
};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::mmio::long_mode::{LongModeMmioBootLayout, LongModeMmioPageMapping};
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use std::io;

const RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const FIRST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
const FIRST_STACK: u64 = 0x1f_f000;
const AP_TRAMPOLINE: GuestPhysAddr = GuestPhysAddr::new(0x8000);
const SHARED_MARKER: GuestPhysAddr = GuestPhysAddr::new(0x9000);
const SHARED_MARKER_VALUE: u8 = b'K';
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const X86_RFLAGS_INTERRUPT_ENABLE: u64 = 1 << 9;
const X86_CR0_PROTECTED_MODE_ENABLE: u64 = 1;
const KVM_MP_STATE_RUNNABLE: u32 = 0;
const KVM_MP_STATE_UNINITIALIZED: u32 = 1;
const SIPI_CS_SELECTOR: u16 = 0x0800;
const SIPI_CS_BASE: u64 = 0x8000;

pub const FIRST_VCPU_ID: VcpuId = VcpuId::BOOT;
pub const SECOND_VCPU_ID: VcpuId = VcpuId::new(1);
pub const LAPIC_VIRTUAL_PAGE: u64 = 0x50_0000;
pub const LAPIC_GPA: u64 = 0xfee0_0000;
pub const TARGET_APIC_ID: u8 = 1;
pub const SIPI_VECTOR: u8 = 0x08;
pub const ICR_HIGH_VALUE: u32 = (TARGET_APIC_ID as u32) << 24;
pub const INIT_ASSERT_VALUE: u32 = 0x0000_c500;
pub const INIT_DEASSERT_VALUE: u32 = 0x0000_8500;
pub const SIPI_VALUE: u32 = 0x0000_0600 | SIPI_VECTOR as u32;
pub const FIRST_PROOF: &[u8; 6] = b"0IDSMD";
pub const SECOND_PROOF: &[u8; 3] = b"APD";
pub const AP_LONG_MODE_PROOF: &[u8; 4] = b"ALPD";
pub const AP_LONG_MODE_STACK: u64 = 0x1e_f000;
pub const AP_LONG_MODE_GDT: GuestPhysAddr = GuestPhysAddr::new(0x7000);
pub const AP_LONG_MODE_GDTR: GuestPhysAddr = GuestPhysAddr::new(0x7020);
pub const AP_LONG_MODE_CODE_SELECTOR: u16 = 0x0008;
pub const AP_LONG_MODE_DATA_SELECTOR: u16 = 0x0010;
pub const AP_LONG_MODE_GDT_LIMIT: u16 = 23;

const AP_LONG_MODE_GDT_BYTES: [u8; 24] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xff, 0xff, 0x00, 0x00, 0x00, 0x9a, 0xaf, 0x00,
    0xff, 0xff, 0x00, 0x00, 0x00, 0x92, 0xcf, 0x00,
];
const AP_LONG_MODE_GDTR_BYTES: [u8; 6] = [0x17, 0x00, 0x00, 0x70, 0x00, 0x00];

#[rustfmt::skip]
const FIRST_GUEST_BYTES: [u8; 97] = [
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

#[rustfmt::skip]
const AP_TRAMPOLINE_BYTES: [u8; 27] = [
    0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0,
    0xb0, b'A', 0xe6, 0xe9,
    0xb0, SHARED_MARKER_VALUE, 0xa2, 0x00, 0x90,
    0xb0, b'P', 0xe6, 0xe9,
    0xb0, b'D', 0xe6, 0xe9,
    0xf4,
];

// Assembled with GNU as/ld at VMA 0x8000. The 16-bit prefix establishes PAE, CR3, EFER.LME
// and CR0.PE|PG from guest code, then far-jumps through selector 0x08 into the 64-bit suffix.
#[rustfmt::skip]
const AP_LONG_MODE_TRAMPOLINE_BYTES: [u8; 121] = [
    0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0, 0xb0, 0x41, 0xe6,
    0xe9, 0x66, 0x0f, 0x01, 0x16, 0x20, 0x70, 0x0f, 0x20, 0xe0, 0x66, 0x83,
    0xc8, 0x20, 0x0f, 0x22, 0xe0, 0x66, 0xb8, 0x00, 0x10, 0x00, 0x00, 0x0f,
    0x22, 0xd8, 0x66, 0xb9, 0x80, 0x00, 0x00, 0xc0, 0x0f, 0x32, 0x66, 0x0d,
    0x00, 0x01, 0x00, 0x00, 0x0f, 0x30, 0x0f, 0x20, 0xc0, 0x66, 0x0d, 0x01,
    0x00, 0x00, 0x80, 0x0f, 0x22, 0xc0, 0x66, 0xea, 0x4a, 0x80, 0x00, 0x00,
    0x08, 0x00, 0x66, 0xb8, 0x10, 0x00, 0x8e, 0xd0, 0x8e, 0xd8, 0x8e, 0xc0,
    0x48, 0xbc, 0x00, 0xf0, 0x1e, 0x00, 0x00, 0x00, 0x00, 0x00, 0xb0, 0x4c,
    0xe6, 0xe9, 0x48, 0xbb, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xb0, 0x4b, 0x88, 0x03, 0xb0, 0x50, 0xe6, 0xe9, 0xb0, 0x44, 0xe6, 0xe9,
    0xf4,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApExecutionMode {
    RealModeMarker,
    GuestLongMode,
}

impl ApExecutionMode {
    const fn trampoline(self) -> &'static [u8] {
        match self {
            Self::RealModeMarker => &AP_TRAMPOLINE_BYTES,
            Self::GuestLongMode => &AP_LONG_MODE_TRAMPOLINE_BYTES,
        }
    }

    const fn proof(self) -> &'static [u8] {
        match self {
            Self::RealModeMarker => SECOND_PROOF,
            Self::GuestLongMode => AP_LONG_MODE_PROOF,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ApStartupState {
    mp_state: u32,
    rip: u64,
    cs_selector: u16,
    cs_base: u64,
    cr0: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApLongModeState {
    rsp: u64,
    cs_selector: u16,
    cs_long: u8,
    ss_selector: u16,
    gdt_base: u64,
    gdt_limit: u16,
    cr0: u64,
    cr3: u64,
    cr4: u64,
    efer: u64,
}

impl ApLongModeState {
    #[must_use] pub const fn rsp(self) -> u64 { self.rsp }
    #[must_use] pub const fn cs_selector(self) -> u16 { self.cs_selector }
    #[must_use] pub const fn cs_long(self) -> u8 { self.cs_long }
    #[must_use] pub const fn ss_selector(self) -> u16 { self.ss_selector }
    #[must_use] pub const fn gdt_base(self) -> u64 { self.gdt_base }
    #[must_use] pub const fn gdt_limit(self) -> u16 { self.gdt_limit }
    #[must_use] pub const fn cr0(self) -> u64 { self.cr0 }
    #[must_use] pub const fn cr3(self) -> u64 { self.cr3 }
    #[must_use] pub const fn cr4(self) -> u64 { self.cr4 }
    #[must_use] pub const fn efer(self) -> u64 { self.efer }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuInitSipiResult {
    first_io_exits: Vec<PortIoExit>, second_io_exits: Vec<PortIoExit>, first_proof: Vec<u8>, second_proof: Vec<u8>,
    initial_mp_state: u32, startup_mp_state: u32, startup_rip: u64, startup_cs_selector: u16, startup_cs_base: u64,
    startup_cr0: u64, final_mp_state: u32, ap_completion_rflags: u64, shared_marker: u8,
}

impl TwoVcpuInitSipiResult {
    #[must_use] pub fn first_io_exits(&self) -> &[PortIoExit] { &self.first_io_exits }
    #[must_use] pub fn second_io_exits(&self) -> &[PortIoExit] { &self.second_io_exits }
    #[must_use] pub fn first_proof(&self) -> &[u8] { &self.first_proof }
    #[must_use] pub fn second_proof(&self) -> &[u8] { &self.second_proof }
    #[must_use] pub const fn initial_mp_state(&self) -> u32 { self.initial_mp_state }
    #[must_use] pub const fn startup_mp_state(&self) -> u32 { self.startup_mp_state }
    #[must_use] pub const fn startup_rip(&self) -> u64 { self.startup_rip }
    #[must_use] pub const fn startup_cs_selector(&self) -> u16 { self.startup_cs_selector }
    #[must_use] pub const fn startup_cs_base(&self) -> u64 { self.startup_cs_base }
    #[must_use] pub const fn startup_cr0(&self) -> u64 { self.startup_cr0 }
    #[must_use] pub const fn final_mp_state(&self) -> u32 { self.final_mp_state }
    #[must_use] pub const fn ap_completion_rflags(&self) -> u64 { self.ap_completion_rflags }
    #[must_use] pub const fn shared_marker(&self) -> u8 { self.shared_marker }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuApLongModeResult {
    first_io_exits: Vec<PortIoExit>, second_io_exits: Vec<PortIoExit>, first_proof: Vec<u8>, second_proof: Vec<u8>,
    initial_mp_state: u32, startup_mp_state: u32, startup_rip: u64, startup_cs_selector: u16, startup_cs_base: u64,
    startup_cr0: u64, final_mp_state: u32, ap_completion_rflags: u64, shared_marker: u8, long_mode: ApLongModeState,
}

impl TwoVcpuApLongModeResult {
    #[must_use] pub fn first_io_exits(&self) -> &[PortIoExit] { &self.first_io_exits }
    #[must_use] pub fn second_io_exits(&self) -> &[PortIoExit] { &self.second_io_exits }
    #[must_use] pub fn first_proof(&self) -> &[u8] { &self.first_proof }
    #[must_use] pub fn second_proof(&self) -> &[u8] { &self.second_proof }
    #[must_use] pub const fn initial_mp_state(&self) -> u32 { self.initial_mp_state }
    #[must_use] pub const fn startup_mp_state(&self) -> u32 { self.startup_mp_state }
    #[must_use] pub const fn startup_rip(&self) -> u64 { self.startup_rip }
    #[must_use] pub const fn startup_cs_selector(&self) -> u16 { self.startup_cs_selector }
    #[must_use] pub const fn startup_cs_base(&self) -> u64 { self.startup_cs_base }
    #[must_use] pub const fn startup_cr0(&self) -> u64 { self.startup_cr0 }
    #[must_use] pub const fn final_mp_state(&self) -> u32 { self.final_mp_state }
    #[must_use] pub const fn ap_completion_rflags(&self) -> u64 { self.ap_completion_rflags }
    #[must_use] pub const fn shared_marker(&self) -> u8 { self.shared_marker }
    #[must_use] pub const fn long_mode_state(&self) -> ApLongModeState { self.long_mode }
}

#[derive(Debug)]
struct ApWorkerResult {
    io_exits: Vec<PortIoExit>, proof: Vec<u8>, startup: ApStartupState, final_mp_state: u32,
    completion_rflags: u64, long_mode: Option<ApLongModeState>,
}

#[derive(Debug)]
struct StartupOutcome {
    first_io_exits: Vec<PortIoExit>, second: ApWorkerResult, first_proof: Vec<u8>, initial_mp_state: u32, shared_marker: u8,
}

pub fn run_two_vcpu_init_sipi() -> Result<TwoVcpuInitSipiResult, Error> {
    let outcome = run_two_vcpu_startup(ApExecutionMode::RealModeMarker)?;
    Ok(TwoVcpuInitSipiResult {
        first_io_exits: outcome.first_io_exits, second_io_exits: outcome.second.io_exits,
        first_proof: outcome.first_proof, second_proof: outcome.second.proof,
        initial_mp_state: outcome.initial_mp_state, startup_mp_state: outcome.second.startup.mp_state,
        startup_rip: outcome.second.startup.rip, startup_cs_selector: outcome.second.startup.cs_selector,
        startup_cs_base: outcome.second.startup.cs_base, startup_cr0: outcome.second.startup.cr0,
        final_mp_state: outcome.second.final_mp_state, ap_completion_rflags: outcome.second.completion_rflags,
        shared_marker: outcome.shared_marker,
    })
}

pub fn run_two_vcpu_ap_long_mode() -> Result<TwoVcpuApLongModeResult, Error> {
    let outcome = run_two_vcpu_startup(ApExecutionMode::GuestLongMode)?;
    let long_mode = outcome.second.long_mode.ok_or_else(|| verification_error(
        SECOND_VCPU_ID, "AP long-mode result", "guest long-mode execution did not produce a validated long-mode state"
    ))?;
    Ok(TwoVcpuApLongModeResult {
        first_io_exits: outcome.first_io_exits, second_io_exits: outcome.second.io_exits,
        first_proof: outcome.first_proof, second_proof: outcome.second.proof,
        initial_mp_state: outcome.initial_mp_state, startup_mp_state: outcome.second.startup.mp_state,
        startup_rip: outcome.second.startup.rip, startup_cs_selector: outcome.second.startup.cs_selector,
        startup_cs_base: outcome.second.startup.cs_base, startup_cr0: outcome.second.startup.cr0,
        final_mp_state: outcome.second.final_mp_state, ap_completion_rflags: outcome.second.completion_rflags,
        shared_marker: outcome.shared_marker, long_mode,
    })
}

fn run_two_vcpu_startup(mode: ApExecutionMode) -> Result<StartupOutcome, Error> {
    let first_image = FlatGuestImage::new(FIRST_ENTRY, FIRST_ENTRY, &FIRST_GUEST_BYTES)?;
    let trampoline = FlatGuestImage::new(AP_TRAMPOLINE, AP_TRAMPOLINE, mode.trampoline())?;
    let backend = KvmBackend::open()?;
    backend.require_mp_state_capability()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(RAM_BASE, LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeMmioBootLayout::with_device_mappings(
        memory.region(), first_image.entry(), FIRST_STACK,
        vec![LongModeMmioPageMapping::new(LAPIC_VIRTUAL_PAGE, LAPIC_GPA)],
    ).expect("fixed INIT/SIPI BSP LAPIC mapping remains valid");
    layout.install_page_tables(&mut memory)?;
    first_image.load(&mut memory)?;
    trampoline.load(&mut memory)?;
    if mode == ApExecutionMode::GuestLongMode {
        memory.write(AP_LONG_MODE_GDT, &AP_LONG_MODE_GDT_BYTES)?;
        memory.write(AP_LONG_MODE_GDTR, &AP_LONG_MODE_GDTR_BYTES)?;
    }
    memory.write(SHARED_MARKER, &[0])?;
    vm.register_guest_memory(memory)?;

    let mut first_vcpu = vm.create_vcpu(FIRST_VCPU_ID)?;
    let second_vcpu = vm.create_vcpu(SECOND_VCPU_ID)?;
    first_vcpu.initialize_long_mode(layout.boot_layout())?;
    let _ = first_vcpu.configure_legacy_pic_extint()?;
    let initial_mp_state = require_mp_state(&second_vcpu, KVM_MP_STATE_UNINITIALIZED, "INIT/SIPI initial AP MP state")?;

    let mut first_port_io = PortIoBus::with_debug_port();
    let first_zero = run_expected_debug_output(&mut first_vcpu, &mut first_port_io, b'0', "INIT/SIPI BSP pre-INIT barrier")?;
    require_interrupt_disabled_flags(FIRST_VCPU_ID, "INIT/SIPI BSP pre-INIT state", first_vcpu.registers()?.rflags)?;
    let first_init = run_expected_debug_output(&mut first_vcpu, &mut first_port_io, b'I', "INIT/SIPI BSP INIT-assert barrier")?;
    let first_deassert = run_expected_debug_output(&mut first_vcpu, &mut first_port_io, b'D', "INIT/SIPI BSP INIT-deassert barrier")?;
    let first_sipi = run_expected_debug_output(&mut first_vcpu, &mut first_port_io, b'S', "INIT/SIPI BSP SIPI barrier")?;

    let worker = std::thread::spawn(move || -> Result<ApWorkerResult, Error> {
        let mut second_vcpu = second_vcpu;
        let mut port_io = PortIoBus::with_debug_port();
        let startup_state = require_init_sipi_startup_state(&mut second_vcpu)?;
        let mut io_exits = Vec::new();
        for (index, byte) in mode.proof().iter().copied().enumerate() {
            io_exits.push(run_expected_debug_output(
                &mut second_vcpu, &mut port_io, byte,
                if mode == ApExecutionMode::GuestLongMode {
                    ["AP real-mode startup", "AP 64-bit entry", "AP long-mode marker completion", "AP long-mode completion barrier"][index]
                } else {
                    ["INIT/SIPI AP trampoline startup", "INIT/SIPI AP shared-marker completion", "INIT/SIPI AP completion barrier", "unused"][index]
                },
            )?);
        }
        let final_mp_state = require_mp_state(&second_vcpu, KVM_MP_STATE_RUNNABLE, "INIT/SIPI final AP MP state")?;
        let completion_rflags = second_vcpu.registers()?.rflags;
        require_interrupt_disabled_flags(SECOND_VCPU_ID, "INIT/SIPI AP completion state", completion_rflags)?;
        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
        if proof.as_slice() != mode.proof() {
            return Err(verification_error(SECOND_VCPU_ID, "INIT/SIPI AP proof", format!("expected {:?}, got {proof:?}", mode.proof())));
        }
        let long_mode = if mode == ApExecutionMode::GuestLongMode {
            Some(require_ap_long_mode_state(&second_vcpu)?)
        } else { None };
        Ok(ApWorkerResult { io_exits, proof, startup: startup_state, final_mp_state, completion_rflags, long_mode })
    });
    let second = worker.join().map_err(|_| verification_error(SECOND_VCPU_ID, "INIT/SIPI AP worker join", "secondary vCPU worker panicked"))??;

    let mut shared_marker = [0_u8; 1];
    vm.guest_memory().expect("registered INIT/SIPI guest memory remains VM-owned").read(SHARED_MARKER, &mut shared_marker)?;
    if shared_marker[0] != SHARED_MARKER_VALUE {
        return Err(verification_error(SECOND_VCPU_ID, "INIT/SIPI shared-memory handoff", format!("expected AP marker {SHARED_MARKER_VALUE:#x}, got {:#x}", shared_marker[0])));
    }
    let first_marker = run_expected_debug_output(&mut first_vcpu, &mut first_port_io, b'M', "INIT/SIPI BSP shared-marker observation")?;
    let first_completion = run_expected_debug_output(&mut first_vcpu, &mut first_port_io, b'D', "INIT/SIPI BSP completion barrier")?;
    require_interrupt_disabled_flags(FIRST_VCPU_ID, "INIT/SIPI BSP completion state", first_vcpu.registers()?.rflags)?;
    let first_proof = first_port_io.debug_output().unwrap_or(&[]).to_vec();
    if first_proof.as_slice() != FIRST_PROOF {
        return Err(verification_error(FIRST_VCPU_ID, "INIT/SIPI BSP proof", format!("expected {:?}, got {first_proof:?}", FIRST_PROOF)));
    }
    Ok(StartupOutcome {
        first_io_exits: vec![first_zero, first_init, first_deassert, first_sipi, first_marker, first_completion],
        second, first_proof, initial_mp_state, shared_marker: shared_marker[0],
    })
}

fn require_ap_long_mode_state(vcpu: &Vcpu) -> Result<ApLongModeState, Error> {
    let registers = vcpu.registers()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let cs = special.cs();
    let ss = special.ss();
    let gdt = special.gdt();
    let valid = registers.rsp == AP_LONG_MODE_STACK
        && cs.selector() == AP_LONG_MODE_CODE_SELECTOR && cs.base() == 0 && cs.l() == 1 && cs.db() == 0 && cs.present() == 1
        && ss.selector() == AP_LONG_MODE_DATA_SELECTOR
        && gdt.base() == AP_LONG_MODE_GDT.get() && gdt.limit() == AP_LONG_MODE_GDT_LIMIT
        && special.cr0() & LONG_MODE_CR0_REQUIRED_BITS == LONG_MODE_CR0_REQUIRED_BITS
        && special.cr4() & LONG_MODE_CR4_REQUIRED_BITS == LONG_MODE_CR4_REQUIRED_BITS
        && special.efer() & LONG_MODE_EFER_REQUIRED_BITS == LONG_MODE_EFER_REQUIRED_BITS
        && special.cr3() == LONG_MODE_PML4_ADDR.get();
    if !valid {
        return Err(verification_error(vcpu.id(), "AP guest-driven long-mode state", format!(
            "expected rsp={AP_LONG_MODE_STACK:#x}, cs={AP_LONG_MODE_CODE_SELECTOR:#x}/L=1, ss={AP_LONG_MODE_DATA_SELECTOR:#x}, gdt={:#x}/{AP_LONG_MODE_GDT_LIMIT:#x}, cr3={:#x}, CR0/CR4/EFER long-mode bits; got rsp={:#x}, cs={:#x} base={:#x} L={} DB={} P={}, ss={:#x}, gdt={:#x}/{:#x}, cr0={:#x}, cr3={:#x}, cr4={:#x}, efer={:#x}",
            AP_LONG_MODE_GDT.get(), LONG_MODE_PML4_ADDR.get(), registers.rsp, cs.selector(), cs.base(), cs.l(), cs.db(), cs.present(), ss.selector(), gdt.base(), gdt.limit(), special.cr0(), special.cr3(), special.cr4(), special.efer()
        )));
    }
    Ok(ApLongModeState {
        rsp: registers.rsp, cs_selector: cs.selector(), cs_long: cs.l(), ss_selector: ss.selector(),
        gdt_base: gdt.base(), gdt_limit: gdt.limit(), cr0: special.cr0(), cr3: special.cr3(), cr4: special.cr4(), efer: special.efer(),
    })
}

fn require_init_sipi_startup_state(vcpu: &mut Vcpu) -> Result<ApStartupState, Error> {
    let mp_state = vcpu.accept_init_sipi_startup_handoff()?;
    let registers = vcpu.registers()?;
    let special = vcpu.capture_special_register_snapshot()?;
    let cs = special.cs();
    if registers.rip != 0 || cs.selector() != SIPI_CS_SELECTOR || cs.base() != SIPI_CS_BASE || special.cr0() & X86_CR0_PROTECTED_MODE_ENABLE != 0 {
        return Err(verification_error(vcpu.id(), "INIT/SIPI AP startup architectural state after KVM_RUN EAGAIN", format!(
            "expected MP={KVM_MP_STATE_RUNNABLE}, RIP=0, CS={SIPI_CS_SELECTOR:#x} base={SIPI_CS_BASE:#x}, CR0.PE=0; got MP={mp_state}, RIP={:#x}, CS={:#x} base={:#x}, CR0={:#x}", registers.rip, cs.selector(), cs.base(), special.cr0()
        )));
    }
    Ok(ApStartupState { mp_state, rip: registers.rip, cs_selector: cs.selector(), cs_base: cs.base(), cr0: special.cr0() })
}

fn require_mp_state(vcpu: &Vcpu, expected: u32, stage: &'static str) -> Result<u32, Error> {
    let observed = vcpu.multiprocessing_state_raw()?;
    if observed != expected { return Err(verification_error(vcpu.id(), stage, format!("expected KVM MP state {expected}, got {observed}"))); }
    Ok(observed)
}

fn run_expected_debug_output(vcpu: &mut Vcpu, port_io: &mut PortIoBus, expected: u8, stage: &'static str) -> Result<PortIoExit, Error> {
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Io { return Err(verification_error(vcpu.id(), stage, format!("expected KVM_EXIT_IO, got {exit:?}"))); }
    let io_exit = vcpu.port_io_exit()?;
    if io_exit.direction() != PortIoDirection::Out || io_exit.port() != DEBUG_PORT || io_exit.size() != 1 || io_exit.count() != 1 || io_exit.output_data() != [expected] {
        return Err(verification_error(vcpu.id(), stage, format!("unexpected debug output exit: {io_exit:?}; expected byte {expected:#x}")));
    }
    if port_io.dispatch(&io_exit)? != PortIoService::Output { return Err(verification_error(vcpu.id(), stage, "debug output unexpectedly requested an input response")); }
    Ok(io_exit)
}

fn require_interrupt_disabled_flags(id: VcpuId, stage: &'static str, rflags: u64) -> Result<(), Error> {
    if rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT || rflags & X86_RFLAGS_INTERRUPT_ENABLE != 0 {
        return Err(verification_error(id, stage, format!("expected architectural bit1 set and IF clear, got RFLAGS {rflags:#x}")));
    }
    Ok(())
}

fn verification_error(id: VcpuId, operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation { id: id.get(), operation, source: io::Error::new(io::ErrorKind::InvalidData, detail.into()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_sipi_constants_match_xapic_startup_encoding() {
        assert_eq!(TARGET_APIC_ID, SECOND_VCPU_ID.get() as u8);
        assert_eq!(SIPI_VECTOR, 0x08);
        assert_eq!(AP_TRAMPOLINE.get(), u64::from(SIPI_VECTOR) << 12);
        assert_eq!(SIPI_CS_SELECTOR, 0x0800);
        assert_eq!(SIPI_CS_BASE, AP_TRAMPOLINE.get());
        assert_eq!(ICR_HIGH_VALUE, 0x0100_0000);
        assert_eq!(INIT_ASSERT_VALUE, 0x0000_c500);
        assert_eq!(INIT_DEASSERT_VALUE, 0x0000_8500);
        assert_eq!(SIPI_VALUE, 0x0000_0608);
        assert_eq!(KVM_MP_STATE_UNINITIALIZED, 1);
        assert_eq!(KVM_MP_STATE_RUNNABLE, 0);
        assert_eq!(&FIRST_GUEST_BYTES[15..25], &[0xc7, 0x83, 0x10, 0x03, 0, 0, 0, 0, 0, 1]);
        assert_eq!(&FIRST_GUEST_BYTES[25..35], &[0xc7, 0x83, 0, 0x03, 0, 0, 0, 0xc5, 0, 0]);
        assert_eq!(&FIRST_GUEST_BYTES[39..49], &[0xc7, 0x83, 0, 0x03, 0, 0, 0, 0x85, 0, 0]);
        assert_eq!(&FIRST_GUEST_BYTES[53..63], &[0xc7, 0x83, 0, 0x03, 0, 0, 0x08, 0x06, 0, 0]);
    }

    #[test]
    fn real_mode_ap_trampoline_preserves_historical_proof() {
        assert_eq!(AP_TRAMPOLINE.get(), 0x8000);
        assert_eq!(SHARED_MARKER.get(), 0x9000);
        assert_eq!(&AP_TRAMPOLINE_BYTES[..9], &[0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0]);
        assert_eq!(&AP_TRAMPOLINE_BYTES[13..18], &[0xb0, SHARED_MARKER_VALUE, 0xa2, 0x00, 0x90]);
        assert_eq!(SECOND_PROOF, b"APD");
    }

    #[test]
    fn ap_long_mode_tables_and_machine_code_match_transition_contract() {
        assert_eq!(AP_LONG_MODE_GDT_BYTES[8..16], [0xff, 0xff, 0, 0, 0, 0x9a, 0xaf, 0]);
        assert_eq!(AP_LONG_MODE_GDT_BYTES[16..24], [0xff, 0xff, 0, 0, 0, 0x92, 0xcf, 0]);
        assert_eq!(AP_LONG_MODE_GDTR_BYTES, [0x17, 0, 0, 0x70, 0, 0]);
        assert_eq!(&AP_LONG_MODE_TRAMPOLINE_BYTES[..13], &[0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0, 0xb0, b'A', 0xe6, 0xe9]);
        assert_eq!(&AP_LONG_MODE_TRAMPOLINE_BYTES[66..74], &[0x66, 0xea, 0x4a, 0x80, 0, 0, 0x08, 0]);
        assert_eq!(AP_LONG_MODE_PROOF, b"ALPD");
        assert_ne!(AP_LONG_MODE_STACK, FIRST_STACK);
    }

    #[test]
    fn bsp_proof_covers_init_deassert_sipi_and_shared_handoff() {
        assert_eq!(FIRST_PROOF, b"0IDSMD");
        assert_eq!(&FIRST_GUEST_BYTES[81..83], &[0x75, 0x09]);
        assert_eq!(&FIRST_GUEST_BYTES[83..91], &[0xb0, b'M', 0xe6, 0xe9, 0xb0, b'D', 0xe6, 0xe9]);
        assert_eq!(&FIRST_GUEST_BYTES[92..96], &[0xb0, b'F', 0xe6, 0xe9]);
    }
}
