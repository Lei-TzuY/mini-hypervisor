use super::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::error::{Error, HostEnvironmentError};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
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
const KVM_MP_STATE_INIT_RECEIVED: u32 = 2;

pub const FIRST_VCPU_ID: VcpuId = VcpuId::BOOT;
pub const SECOND_VCPU_ID: VcpuId = VcpuId::new(1);
pub const LAPIC_VIRTUAL_PAGE: u64 = 0x50_0000;
pub const LAPIC_GPA: u64 = 0xfee0_0000;
pub const LAPIC_ICR_LOW_OFFSET: u32 = 0x300;
pub const LAPIC_ICR_HIGH_OFFSET: u32 = 0x310;
pub const TARGET_APIC_ID: u8 = 1;
pub const SIPI_VECTOR: u8 = 0x08;
pub const ICR_HIGH_VALUE: u32 = (TARGET_APIC_ID as u32) << 24;
pub const INIT_ASSERT_VALUE: u32 = 0x0000_c500;
pub const INIT_DEASSERT_VALUE: u32 = 0x0000_8500;
pub const SIPI_VALUE: u32 = 0x0000_0600 | SIPI_VECTOR as u32;
pub const FIRST_PROOF: &[u8; 6] = b"0IDSMD";
pub const SECOND_PROOF: &[u8; 3] = b"APD";

const FIRST_GUEST_BYTES: [u8; 97] = [
    0xfa, // cli: keep BSP interrupt state out of the AP-startup proof
    0x48,
    0xbb,
    0x00,
    0x00,
    0x50,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00, // movabs LAPIC alias, %rbx
    0xb0,
    b'0',
    0xe6,
    0xe9, // pre-INIT synchronization barrier
    0xc7,
    0x83,
    0x10,
    0x03,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x01, // ICR high: destination APIC ID 1
    0xc7,
    0x83,
    0x00,
    0x03,
    0x00,
    0x00,
    0x00,
    0xc5,
    0x00,
    0x00, // ICR low: INIT assert, level-triggered
    0xb0,
    b'I',
    0xe6,
    0xe9, // INIT-assert completion barrier
    0xc7,
    0x83,
    0x00,
    0x03,
    0x00,
    0x00,
    0x00,
    0x85,
    0x00,
    0x00, // ICR low: INIT deassert, level-triggered
    0xb0,
    b'D',
    0xe6,
    0xe9, // INIT-deassert completion barrier
    0xc7,
    0x83,
    0x00,
    0x03,
    0x00,
    0x00,
    SIPI_VECTOR,
    0x06,
    0x00,
    0x00, // ICR low: STARTUP IPI vector 0x08
    0xb0,
    b'S',
    0xe6,
    0xe9, // SIPI completion barrier
    0x48,
    0xb9,
    0x00,
    0x90,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00, // movabs shared marker GPA 0x9000, %rcx
    0x8a,
    0x01, // mov (%rcx), %al
    0x3c,
    SHARED_MARKER_VALUE, // cmp $'K', %al
    0x75,
    0x09, // jne failure
    0xb0,
    b'M',
    0xe6,
    0xe9, // AP-to-BSP shared-memory handoff observed
    0xb0,
    b'D',
    0xe6,
    0xe9, // BSP completion barrier
    0xf4, // not re-entered after the successful completion barrier
    0xb0,
    b'F',
    0xe6,
    0xe9,
    0xf4, // failure path
];

const AP_TRAMPOLINE_BYTES: [u8; 27] = [
    0xfa, // cli
    0x31,
    0xc0, // xor ax, ax
    0x8e,
    0xd8, // mov ds, ax
    0x8e,
    0xc0, // mov es, ax
    0x8e,
    0xd0, // mov ss, ax
    0xb0,
    b'A',
    0xe6,
    0xe9, // real-mode AP startup identity
    0xb0,
    SHARED_MARKER_VALUE, // marker value
    0xa2,
    0x00,
    0x90, // mov [0x9000], al using 16-bit moffs
    0xb0,
    b'P',
    0xe6,
    0xe9, // shared-memory write completed
    0xb0,
    b'D',
    0xe6,
    0xe9, // AP completion barrier
    0xf4, // not re-entered after D
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuInitSipiResult {
    first_io_exits: Vec<PortIoExit>,
    second_io_exits: Vec<PortIoExit>,
    first_proof: Vec<u8>,
    second_proof: Vec<u8>,
    initial_mp_state: u32,
    post_init_mp_state: u32,
    post_deassert_mp_state: u32,
    post_sipi_mp_state: u32,
    ap_start_rip: u64,
    ap_start_cs_selector: u16,
    ap_start_cs_base: u64,
    ap_start_cr0: u64,
    ap_completion_rflags: u64,
    shared_marker: u8,
}

impl TwoVcpuInitSipiResult {
    #[must_use]
    pub fn first_io_exits(&self) -> &[PortIoExit] {
        &self.first_io_exits
    }

    #[must_use]
    pub fn second_io_exits(&self) -> &[PortIoExit] {
        &self.second_io_exits
    }

    #[must_use]
    pub fn first_proof(&self) -> &[u8] {
        &self.first_proof
    }

    #[must_use]
    pub fn second_proof(&self) -> &[u8] {
        &self.second_proof
    }

    #[must_use]
    pub const fn initial_mp_state(&self) -> u32 {
        self.initial_mp_state
    }

    #[must_use]
    pub const fn post_init_mp_state(&self) -> u32 {
        self.post_init_mp_state
    }

    #[must_use]
    pub const fn post_deassert_mp_state(&self) -> u32 {
        self.post_deassert_mp_state
    }

    #[must_use]
    pub const fn post_sipi_mp_state(&self) -> u32 {
        self.post_sipi_mp_state
    }

    #[must_use]
    pub const fn ap_start_rip(&self) -> u64 {
        self.ap_start_rip
    }

    #[must_use]
    pub const fn ap_start_cs_selector(&self) -> u16 {
        self.ap_start_cs_selector
    }

    #[must_use]
    pub const fn ap_start_cs_base(&self) -> u64 {
        self.ap_start_cs_base
    }

    #[must_use]
    pub const fn ap_start_cr0(&self) -> u64 {
        self.ap_start_cr0
    }

    #[must_use]
    pub const fn ap_completion_rflags(&self) -> u64 {
        self.ap_completion_rflags
    }

    #[must_use]
    pub const fn shared_marker(&self) -> u8 {
        self.shared_marker
    }
}

#[derive(Debug)]
struct ApWorkerResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    completion_rflags: u64,
}

pub fn run_two_vcpu_init_sipi() -> Result<TwoVcpuInitSipiResult, Error> {
    let first_image = FlatGuestImage::new(FIRST_ENTRY, FIRST_ENTRY, &FIRST_GUEST_BYTES)?;
    let trampoline = FlatGuestImage::new(AP_TRAMPOLINE, AP_TRAMPOLINE, &AP_TRAMPOLINE_BYTES)?;

    let backend = KvmBackend::open()?;
    backend.require_mp_state_capability()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(RAM_BASE, LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeMmioBootLayout::with_device_mappings(
        memory.region(),
        first_image.entry(),
        FIRST_STACK,
        vec![LongModeMmioPageMapping::new(LAPIC_VIRTUAL_PAGE, LAPIC_GPA)],
    )
    .expect("fixed INIT/SIPI BSP LAPIC mapping remains valid");
    layout.install_page_tables(&mut memory)?;
    first_image.load(&mut memory)?;
    trampoline.load(&mut memory)?;
    memory.write(SHARED_MARKER, &[0])?;
    vm.register_guest_memory(memory)?;

    let mut first_vcpu = vm.create_vcpu(FIRST_VCPU_ID)?;
    let mut second_vcpu = vm.create_vcpu(SECOND_VCPU_ID)?;
    first_vcpu.initialize_long_mode(layout.boot_layout())?;
    let _ = first_vcpu.configure_legacy_pic_extint()?;

    // This is the central milestone invariant: userspace never sets the AP to RUNNABLE. The AP
    // remains in KVM's reset UNINITIALIZED state until the BSP's guest-originated INIT/SIPI traffic
    // advances the in-kernel local-APIC state machine.
    let initial_mp_state = require_mp_state(
        &second_vcpu,
        KVM_MP_STATE_UNINITIALIZED,
        "INIT/SIPI initial AP MP state",
    )?;

    let mut first_port_io = PortIoBus::with_debug_port();
    let first_zero = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'0',
        "INIT/SIPI BSP pre-INIT barrier",
    )?;
    require_interrupt_disabled_flags(
        FIRST_VCPU_ID,
        "INIT/SIPI BSP pre-INIT state",
        first_vcpu.registers()?.rflags,
    )?;

    let first_init = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'I',
        "INIT/SIPI BSP INIT-assert barrier",
    )?;
    let post_init_mp_state = require_mp_state(
        &second_vcpu,
        KVM_MP_STATE_INIT_RECEIVED,
        "INIT/SIPI AP state after INIT assert",
    )?;

    let first_deassert = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'D',
        "INIT/SIPI BSP INIT-deassert barrier",
    )?;
    let post_deassert_mp_state = require_mp_state(
        &second_vcpu,
        KVM_MP_STATE_INIT_RECEIVED,
        "INIT/SIPI AP state after INIT deassert",
    )?;

    let first_sipi = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'S',
        "INIT/SIPI BSP SIPI barrier",
    )?;
    let post_sipi_mp_state = require_mp_state(
        &second_vcpu,
        KVM_MP_STATE_RUNNABLE,
        "INIT/SIPI AP state after SIPI",
    )?;

    let ap_start_regs = second_vcpu.registers()?;
    let ap_start_sregs = second_vcpu.capture_special_register_snapshot()?;
    let expected_cs_selector = u16::from(SIPI_VECTOR) << 8;
    let expected_cs_base = u64::from(SIPI_VECTOR) << 12;
    if ap_start_regs.rip != 0
        || ap_start_sregs.cs().selector() != expected_cs_selector
        || ap_start_sregs.cs().base() != expected_cs_base
        || ap_start_sregs.cr0() & X86_CR0_PROTECTED_MODE_ENABLE != 0
    {
        return Err(verification_error(
            SECOND_VCPU_ID,
            "INIT/SIPI AP startup state",
            format!(
                "expected RIP=0, CS.selector={expected_cs_selector:#x}, CS.base={expected_cs_base:#x}, CR0.PE=0; got RIP={:#x}, selector={:#x}, base={:#x}, CR0={:#x}",
                ap_start_regs.rip,
                ap_start_sregs.cs().selector(),
                ap_start_sregs.cs().base(),
                ap_start_sregs.cr0()
            ),
        ));
    }

    // Only after KVM has accepted the SIPI and userspace has verified the exact startup state does
    // ownership move to the AP worker. There is still exactly one userspace owner of this Vcpu.
    let worker = std::thread::spawn(move || -> Result<ApWorkerResult, Error> {
        let mut port_io = PortIoBus::with_debug_port();
        let startup = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'A',
            "INIT/SIPI AP trampoline startup",
        )?;
        let marker = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'P',
            "INIT/SIPI AP shared-marker completion",
        )?;
        let completion = run_expected_debug_output(
            &mut second_vcpu,
            &mut port_io,
            b'D',
            "INIT/SIPI AP completion barrier",
        )?;
        let completion_rflags = second_vcpu.registers()?.rflags;
        require_interrupt_disabled_flags(
            SECOND_VCPU_ID,
            "INIT/SIPI AP completion state",
            completion_rflags,
        )?;
        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
        if proof.as_slice() != SECOND_PROOF {
            return Err(verification_error(
                SECOND_VCPU_ID,
                "INIT/SIPI AP trampoline proof",
                format!("expected {:?}, got {proof:?}", SECOND_PROOF),
            ));
        }
        Ok(ApWorkerResult {
            io_exits: vec![startup, marker, completion],
            proof,
            completion_rflags,
        })
    });

    let second = worker.join().map_err(|_| {
        verification_error(
            SECOND_VCPU_ID,
            "INIT/SIPI AP worker join",
            "secondary vCPU worker panicked",
        )
    })??;

    let mut shared_marker = [0_u8; 1];
    vm.guest_memory()
        .expect("registered INIT/SIPI guest memory remains VM-owned")
        .read(SHARED_MARKER, &mut shared_marker)?;
    if shared_marker[0] != SHARED_MARKER_VALUE {
        return Err(verification_error(
            SECOND_VCPU_ID,
            "INIT/SIPI shared-memory handoff",
            format!(
                "expected AP marker {SHARED_MARKER_VALUE:#x}, got {:#x}",
                shared_marker[0]
            ),
        ));
    }

    let first_marker = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'M',
        "INIT/SIPI BSP shared-marker observation",
    )?;
    let first_completion = run_expected_debug_output(
        &mut first_vcpu,
        &mut first_port_io,
        b'D',
        "INIT/SIPI BSP completion barrier",
    )?;
    require_interrupt_disabled_flags(
        FIRST_VCPU_ID,
        "INIT/SIPI BSP completion state",
        first_vcpu.registers()?.rflags,
    )?;
    let first_proof = first_port_io.debug_output().unwrap_or(&[]).to_vec();
    if first_proof.as_slice() != FIRST_PROOF {
        return Err(verification_error(
            FIRST_VCPU_ID,
            "INIT/SIPI BSP proof",
            format!("expected {:?}, got {first_proof:?}", FIRST_PROOF),
        ));
    }

    Ok(TwoVcpuInitSipiResult {
        first_io_exits: vec![
            first_zero,
            first_init,
            first_deassert,
            first_sipi,
            first_marker,
            first_completion,
        ],
        second_io_exits: second.io_exits,
        first_proof,
        second_proof: second.proof,
        initial_mp_state,
        post_init_mp_state,
        post_deassert_mp_state,
        post_sipi_mp_state,
        ap_start_rip: ap_start_regs.rip,
        ap_start_cs_selector: ap_start_sregs.cs().selector(),
        ap_start_cs_base: ap_start_sregs.cs().base(),
        ap_start_cr0: ap_start_sregs.cr0(),
        ap_completion_rflags: second.completion_rflags,
        shared_marker: shared_marker[0],
    })
}

fn require_mp_state(vcpu: &Vcpu, expected: u32, stage: &'static str) -> Result<u32, Error> {
    let observed = vcpu.multiprocessing_state_raw()?;
    if observed != expected {
        return Err(verification_error(
            vcpu.id(),
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
            vcpu.id(),
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
            vcpu.id(),
            stage,
            format!("unexpected debug output exit: {io_exit:?}; expected byte {expected:#x}"),
        ));
    }
    if port_io.dispatch(&io_exit)? != PortIoService::Output {
        return Err(verification_error(
            vcpu.id(),
            stage,
            "debug output unexpectedly requested an input response",
        ));
    }
    Ok(io_exit)
}

fn require_interrupt_disabled_flags(
    id: VcpuId,
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

fn verification_error(id: VcpuId, operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: id.get(),
        operation,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_sipi_constants_match_xapic_startup_encoding() {
        assert_eq!(TARGET_APIC_ID, SECOND_VCPU_ID.get() as u8);
        assert_eq!(SIPI_VECTOR, 0x08);
        assert_eq!(AP_TRAMPOLINE.get(), u64::from(SIPI_VECTOR) << 12);
        assert_eq!(ICR_HIGH_VALUE, 0x0100_0000);
        assert_eq!(INIT_ASSERT_VALUE, 0x0000_c500);
        assert_eq!(INIT_DEASSERT_VALUE, 0x0000_8500);
        assert_eq!(SIPI_VALUE, 0x0000_0608);
        assert_eq!(KVM_MP_STATE_UNINITIALIZED, 1);
        assert_eq!(KVM_MP_STATE_INIT_RECEIVED, 2);
        assert_eq!(KVM_MP_STATE_RUNNABLE, 0);
        assert_eq!(
            &FIRST_GUEST_BYTES[15..25],
            &[0xc7, 0x83, 0x10, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]
        );
        assert_eq!(
            &FIRST_GUEST_BYTES[25..35],
            &[0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x00, 0xc5, 0x00, 0x00]
        );
        assert_eq!(
            &FIRST_GUEST_BYTES[39..49],
            &[0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x00, 0x85, 0x00, 0x00]
        );
        assert_eq!(
            &FIRST_GUEST_BYTES[53..63],
            &[0xc7, 0x83, 0x00, 0x03, 0x00, 0x00, 0x08, 0x06, 0x00, 0x00]
        );
    }

    #[test]
    fn ap_trampoline_is_real_mode_and_writes_shared_marker() {
        assert_eq!(AP_TRAMPOLINE.get(), 0x8000);
        assert_eq!(SHARED_MARKER.get(), 0x9000);
        assert_eq!(
            &AP_TRAMPOLINE_BYTES[..9],
            &[0xfa, 0x31, 0xc0, 0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0]
        );
        assert_eq!(
            &AP_TRAMPOLINE_BYTES[13..18],
            &[0xb0, SHARED_MARKER_VALUE, 0xa2, 0x00, 0x90]
        );
        assert_eq!(SECOND_PROOF, b"APD");
    }

    #[test]
    fn bsp_proof_covers_init_deassert_sipi_and_shared_handoff() {
        assert_eq!(FIRST_PROOF, b"0IDSMD");
        assert_eq!(&FIRST_GUEST_BYTES[81..83], &[0x75, 0x09]);
        assert_eq!(
            &FIRST_GUEST_BYTES[83..91],
            &[0xb0, b'M', 0xe6, 0xe9, 0xb0, b'D', 0xe6, 0xe9]
        );
        assert_eq!(&FIRST_GUEST_BYTES[92..96], &[0xb0, b'F', 0xe6, 0xe9]);
    }
}
