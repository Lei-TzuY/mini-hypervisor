#![forbid(unsafe_op_in_unsafe_fn)]

pub mod config;
pub mod copyin;
pub mod copyout;
pub mod error;
pub mod execution;
pub mod interrupt;
pub mod kvm;
pub mod loader;
pub mod long_mode;
pub mod memory;
pub mod mmio;
pub mod mmio_fixture;
pub mod model;
pub mod portio;
pub mod privilege;
pub mod state_snapshot;
pub mod syscall;
pub mod vcpu;
pub mod vmexit;

use config::VmConfig;
use error::{Error, VmExitError};
use execution::{run_vcpu_until_stopped, VmExecutionResult};
use kvm::msr::GuestMsrAccessPolicy;
use kvm::KvmBackend;
use loader::FlatGuestImage;
use long_mode::LongModeBootLayout;
use memory::{GuestMemory, GuestPhysAddr};
use portio::PortIoBus;
use state_snapshot::VcpuStateSnapshotComparison;
use vcpu::{PortIoExit, VcpuId};
use vmexit::VmExitReport;

const LIFECYCLE_RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const LIFECYCLE_RAM_SIZE: u64 = 2 * 1024 * 1024;
const HLT_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const HLT_GUEST_BYTES: [u8; 1] = [0xf4];
const HLT_EXIT_BUDGET: u32 = 1;
const DEBUG_PORT_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const DEBUG_PORT_GUEST_BYTES: [u8; 5] = [0xb0, b'K', 0xe6, 0xe9, 0xf4];
const DEBUG_PORT_EXIT_BUDGET: u32 = 2;
const DEBUG_PORT_INPUT_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const DEBUG_PORT_INPUT_RESULT: GuestPhysAddr = GuestPhysAddr::new(0x2000);
const DEBUG_PORT_INPUT_VALUE: u8 = b'R';
const DEBUG_PORT_INPUT_GUEST_BYTES: [u8; 6] = [0xe4, 0xe9, 0xa2, 0x00, 0x20, 0xf4];
const CPUID_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const CPUID_GUEST_RESULT: GuestPhysAddr = GuestPhysAddr::new(0x2000);
const CPUID_GUEST_BYTES: [u8; 28] = [
    0x66, 0xb8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1
    0x0f, 0xa2, // cpuid
    0x66, 0x89, 0xc8, // mov eax, ecx
    0x66, 0xa3, 0x00, 0x20, // mov [0x2000], eax
    0x66, 0xb8, 0x01, 0x00, 0x00, 0x40, // mov eax, 0x40000001
    0x0f, 0xa2, // cpuid
    0x66, 0xa3, 0x04, 0x20, // mov [0x2004], eax
    0xf4, // hlt
];
const CPUID_EXIT_BUDGET: u32 = 1;
const CPUID1_X2APIC: u32 = 1 << 21;
const CPUID1_TSC_DEADLINE: u32 = 1 << 24;
const KVM_FEATURE_PV_UNHALT: u32 = 1 << 7;
const STATE_REFERENCE_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const STATE_CHANGED_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1200);
const LONG_MODE_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
const LONG_MODE_GUEST_STACK_POINTER: u64 = 0x1f_f000;
const LONG_MODE_GUEST_PROOF: &[u8; 4] = b"LM64";
const LONG_MODE_GUEST_BYTES: [u8; 36] = [
    0x48, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x4c, 0x4d, 0x36, 0x34, // movabs imm64, %rax
    0x48, 0xc1, 0xe8, 0x20, // shr $32, %rax
    0xba, 0xe9, 0x00, 0x00, 0x00, // mov $0xe9, %edx
    0xee, // out %al, %dx  ('L')
    0x48, 0xc1, 0xe8, 0x08, 0xee, // shr $8, %rax; out ('M')
    0x48, 0xc1, 0xe8, 0x08, 0xee, // shr $8, %rax; out ('6')
    0x48, 0xc1, 0xe8, 0x08, 0xee, // shr $8, %rax; out ('4')
    0xf4, // hlt
];
const LONG_MODE_EXIT_BUDGET: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugPortGuestResult {
    io: PortIoExit,
    output: Vec<u8>,
    report: VmExitReport,
}

impl DebugPortGuestResult {
    #[must_use]
    pub fn io(&self) -> &PortIoExit {
        &self.io
    }

    #[must_use]
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    #[must_use]
    pub const fn report(&self) -> VmExitReport {
        self.report
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugPortInputGuestResult {
    io: PortIoExit,
    value: u8,
    report: VmExitReport,
}

impl DebugPortInputGuestResult {
    #[must_use]
    pub fn io(&self) -> &PortIoExit {
        &self.io
    }

    #[must_use]
    pub const fn value(&self) -> u8 {
        self.value
    }

    #[must_use]
    pub const fn report(&self) -> VmExitReport {
        self.report
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuidGuestResult {
    cpuid1_ecx: u32,
    kvm_features_eax: u32,
    report: VmExitReport,
}

impl CpuidGuestResult {
    #[must_use]
    pub const fn cpuid1_ecx(&self) -> u32 {
        self.cpuid1_ecx
    }

    #[must_use]
    pub const fn kvm_features_eax(&self) -> u32 {
        self.kvm_features_eax
    }

    #[must_use]
    pub const fn masked_lapic_features_clear(&self) -> bool {
        masked_lapic_features_clear(self.cpuid1_ecx, self.kvm_features_eax)
    }

    #[must_use]
    pub const fn report(&self) -> VmExitReport {
        self.report
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongModeGuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
}

impl LongModeGuestResult {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSnapshotRoundTripResult {
    changed: VcpuStateSnapshotComparison,
    restored: VcpuStateSnapshotComparison,
}

impl StateSnapshotRoundTripResult {
    #[must_use]
    pub const fn changed(&self) -> &VcpuStateSnapshotComparison {
        &self.changed
    }

    #[must_use]
    pub const fn restored(&self) -> &VcpuStateSnapshotComparison {
        &self.restored
    }
}

pub fn verify_kvm_lifecycle(config: VmConfig) -> Result<(), Error> {
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    debug_assert_eq!(vcpu.id(), VcpuId::BOOT);

    Ok(())
}

pub fn run_state_snapshot_roundtrip(
    config: VmConfig,
) -> Result<StateSnapshotRoundTripResult, Error> {
    let backend = KvmBackend::open()?;
    let vm = backend.create_vm()?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty guest MSR policy is valid by construction");

    vcpu.initialize_real_mode(STATE_REFERENCE_ENTRY)?;
    let reference = vcpu.capture_state_snapshot(&msr_policy)?;

    vcpu.initialize_real_mode(STATE_CHANGED_ENTRY)?;
    let observed_changed = vcpu.capture_state_snapshot(&msr_policy)?;
    let changed = reference.compare(&observed_changed);
    debug_assert!(
        !changed.is_exact_match(),
        "the deterministic state round-trip fixture must mutate the captured state before restore"
    );

    let restored = vcpu.restore_and_verify_state_snapshot(&reference)?;
    Ok(StateSnapshotRoundTripResult { changed, restored })
}

pub fn run_hlt_guest(config: VmConfig) -> Result<VmExitReport, Error> {
    let image = FlatGuestImage::new(HLT_GUEST_ENTRY, HLT_GUEST_ENTRY, &HLT_GUEST_BYTES)?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_real_mode(image.entry())?;
    let mut port_io = PortIoBus::empty();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, HLT_EXIT_BUDGET)?;

    debug_assert_eq!(execution.completed_exits(), 1);
    debug_assert!(execution.io_exits().is_empty());
    Ok(execution.report())
}

pub fn run_debug_port_guest(config: VmConfig) -> Result<DebugPortGuestResult, Error> {
    let image = FlatGuestImage::new(
        DEBUG_PORT_GUEST_ENTRY,
        DEBUG_PORT_GUEST_ENTRY,
        &DEBUG_PORT_GUEST_BYTES,
    )?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_real_mode(image.entry())?;
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, DEBUG_PORT_EXIT_BUDGET)?;
    let io = required_single_io(&execution, "debug-port output")?;

    debug_assert_eq!(execution.completed_exits(), 2);
    let output = port_io.debug_output().unwrap_or(&[]).to_vec();
    Ok(DebugPortGuestResult {
        io,
        output,
        report: execution.report(),
    })
}

pub fn run_debug_port_input_guest(config: VmConfig) -> Result<DebugPortInputGuestResult, Error> {
    let image = FlatGuestImage::new(
        DEBUG_PORT_INPUT_GUEST_ENTRY,
        DEBUG_PORT_INPUT_GUEST_ENTRY,
        &DEBUG_PORT_INPUT_GUEST_BYTES,
    )?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_real_mode(image.entry())?;
    let mut port_io = PortIoBus::with_debug_port_input(DEBUG_PORT_INPUT_VALUE);
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, DEBUG_PORT_EXIT_BUDGET)?;
    let io = required_single_io(&execution, "debug-port input")?;

    debug_assert_eq!(execution.completed_exits(), 2);
    let mut observed = [0_u8; 1];
    vm.guest_memory()
        .expect("registered guest memory remains owned by the VM")
        .read(DEBUG_PORT_INPUT_RESULT, &mut observed)?;

    Ok(DebugPortInputGuestResult {
        io,
        value: observed[0],
        report: execution.report(),
    })
}

pub fn run_cpuid_guest(config: VmConfig) -> Result<CpuidGuestResult, Error> {
    let image = FlatGuestImage::new(CPUID_GUEST_ENTRY, CPUID_GUEST_ENTRY, &CPUID_GUEST_BYTES)?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_real_mode(image.entry())?;
    let mut port_io = PortIoBus::empty();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, CPUID_EXIT_BUDGET)?;

    debug_assert_eq!(execution.completed_exits(), 1);
    debug_assert!(execution.io_exits().is_empty());
    let mut observed = [0_u8; 8];
    vm.guest_memory()
        .expect("registered guest memory remains owned by the VM")
        .read(CPUID_GUEST_RESULT, &mut observed)?;
    let (cpuid1_ecx, kvm_features_eax) = decode_cpuid_guest_result(observed);

    Ok(CpuidGuestResult {
        cpuid1_ecx,
        kvm_features_eax,
        report: execution.report(),
    })
}

pub fn run_long_mode_guest(config: VmConfig) -> Result<LongModeGuestResult, Error> {
    let image = FlatGuestImage::new(
        LONG_MODE_GUEST_ENTRY,
        LONG_MODE_GUEST_ENTRY,
        &LONG_MODE_GUEST_BYTES,
    )?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    let layout = LongModeBootLayout::new(
        memory.region(),
        image.entry(),
        LONG_MODE_GUEST_STACK_POINTER,
    )
    .expect("fixed deterministic long-mode fixture layout remains valid");
    layout.install_page_tables(&mut memory)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode(&layout)?;
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, LONG_MODE_EXIT_BUDGET)?;

    debug_assert_eq!(execution.completed_exits(), LONG_MODE_EXIT_BUDGET);
    debug_assert_eq!(execution.io_exits().len(), LONG_MODE_GUEST_PROOF.len());
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    Ok(LongModeGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
    })
}

fn decode_cpuid_guest_result(observed: [u8; 8]) -> (u32, u32) {
    let cpuid1_ecx = u32::from_le_bytes([observed[0], observed[1], observed[2], observed[3]]);
    let kvm_features_eax = u32::from_le_bytes([observed[4], observed[5], observed[6], observed[7]]);
    (cpuid1_ecx, kvm_features_eax)
}

const fn masked_lapic_features_clear(cpuid1_ecx: u32, kvm_features_eax: u32) -> bool {
    let cpuid1_mask = CPUID1_X2APIC | CPUID1_TSC_DEADLINE;
    cpuid1_ecx & cpuid1_mask == 0 && kvm_features_eax & KVM_FEATURE_PV_UNHALT == 0
}

fn required_single_io(
    execution: &VmExecutionResult,
    stage: &'static str,
) -> Result<PortIoExit, Error> {
    let Some(io) = execution.io_exits().first() else {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage,
            expected_reason: kvm::sys::KVM_EXIT_IO,
            actual_reason: execution.report().exit().reason(),
        }));
    };

    debug_assert_eq!(execution.io_exits().len(), 1);
    Ok(io.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpuid_guest_machine_code_is_stable() {
        assert_eq!(
            CPUID_GUEST_BYTES,
            [
                0x66, 0xb8, 0x01, 0x00, 0x00, 0x00, 0x0f, 0xa2, 0x66, 0x89, 0xc8, 0x66, 0xa3, 0x00,
                0x20, 0x66, 0xb8, 0x01, 0x00, 0x00, 0x40, 0x0f, 0xa2, 0x66, 0xa3, 0x04, 0x20, 0xf4,
            ]
        );
        assert_eq!(CPUID_GUEST_BYTES.len(), 0x1c);
    }

    #[test]
    fn long_mode_guest_machine_code_is_stable() {
        assert_eq!(
            LONG_MODE_GUEST_BYTES,
            [
                0x48, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x4c, 0x4d, 0x36, 0x34, 0x48, 0xc1, 0xe8, 0x20,
                0xba, 0xe9, 0x00, 0x00, 0x00, 0xee, 0x48, 0xc1, 0xe8, 0x08, 0xee, 0x48, 0xc1, 0xe8,
                0x08, 0xee, 0x48, 0xc1, 0xe8, 0x08, 0xee, 0xf4,
            ]
        );
        assert_eq!(LONG_MODE_GUEST_BYTES.len(), 0x24);
        assert_eq!(LONG_MODE_GUEST_PROOF, b"LM64");
    }

    #[test]
    fn decodes_cpuid_guest_result_as_little_endian_words() {
        assert_eq!(
            decode_cpuid_guest_result([0x78, 0x56, 0x34, 0x12, 0xef, 0xcd, 0xab, 0x90]),
            (0x1234_5678, 0x90ab_cdef)
        );
    }

    #[test]
    fn detects_each_lapic_dependent_feature_bit() {
        assert!(masked_lapic_features_clear(0, 0));
        assert!(!masked_lapic_features_clear(CPUID1_X2APIC, 0));
        assert!(!masked_lapic_features_clear(CPUID1_TSC_DEADLINE, 0));
        assert!(!masked_lapic_features_clear(0, KVM_FEATURE_PV_UNHALT));
    }
}
