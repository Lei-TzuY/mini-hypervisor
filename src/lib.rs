#![forbid(unsafe_op_in_unsafe_fn)]

pub mod config;
pub mod error;
pub mod execution;
pub mod kvm;
pub mod loader;
pub mod memory;
pub mod model;
pub mod portio;
pub mod state_snapshot;
pub mod vcpu;
pub mod vmexit;

use config::VmConfig;
use error::{Error, VmExitError};
use execution::{run_vcpu_until_stopped, VmExecutionResult};
use kvm::KvmBackend;
use loader::FlatGuestImage;
use memory::{GuestMemory, GuestPhysAddr};
use portio::PortIoBus;
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
