#![forbid(unsafe_op_in_unsafe_fn)]

pub mod address_space;
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

    let reference = vcpu.capture_state_snapshot(&GuestMsrAccessPolicy::host_introspection())?;

    let mut registers = vcpu.registers()?;
    registers.rip = STATE_CHANGED_ENTRY.get();
    registers.rflags = 0x202;
    vcpu.set_registers(registers)?;

    let changed = vcpu.compare_state_snapshot(&reference)?;
    vcpu.restore_state_snapshot(&reference)?;
    let restored = vcpu.compare_state_snapshot(&reference)?;

    Ok(StateSnapshotRoundTripResult { changed, restored })
}

pub fn run_hlt_guest(config: VmConfig) -> Result<VmExecutionResult, Error> {
    let guest = FlatGuestImage::new(HLT_GUEST_ENTRY, HLT_GUEST_ENTRY, &HLT_GUEST_BYTES)?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    guest.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    let mut port_io = PortIoBus::empty();
    run_vcpu_until_stopped(&mut vcpu, &mut port_io, HLT_EXIT_BUDGET)
}

pub fn run_debug_port_guest(config: VmConfig) -> Result<DebugPortGuestResult, Error> {
    let guest = FlatGuestImage::new(
        DEBUG_PORT_GUEST_ENTRY,
        DEBUG_PORT_GUEST_ENTRY,
        &DEBUG_PORT_GUEST_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    guest.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, DEBUG_PORT_EXIT_BUDGET)?;
    let io = execution
        .io_exits()
        .first()
        .expect("debug-port guest must execute exactly one port-I/O exit")
        .clone();

    Ok(DebugPortGuestResult {
        io,
        output: port_io.debug_output().unwrap_or(&[]).to_vec(),
        report: execution.report(),
    })
}

pub fn run_debug_port_input_guest(config: VmConfig) -> Result<DebugPortInputGuestResult, Error> {
    let guest = FlatGuestImage::new(
        DEBUG_PORT_INPUT_GUEST_ENTRY,
        DEBUG_PORT_INPUT_GUEST_ENTRY,
        &DEBUG_PORT_INPUT_GUEST_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    guest.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    let mut port_io = PortIoBus::with_debug_port_input(DEBUG_PORT_INPUT_VALUE);
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, DEBUG_PORT_EXIT_BUDGET)?;
    let io = execution
        .io_exits()
        .first()
        .expect("debug-port input guest must execute exactly one port-I/O exit")
        .clone();

    let guest_memory = vm
        .guest_memory()
        .expect("registered debug-port input memory remains VM-owned");
    let mut value = [0_u8; 1];
    guest_memory.read(DEBUG_PORT_INPUT_RESULT, &mut value)?;

    Ok(DebugPortInputGuestResult {
        io,
        value: value[0],
        report: execution.report(),
    })
}

pub fn run_cpuid_guest(config: VmConfig) -> Result<CpuidGuestResult, Error> {
    let guest = FlatGuestImage::new(CPUID_GUEST_ENTRY, CPUID_GUEST_ENTRY, &CPUID_GUEST_BYTES)?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    guest.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    let mut port_io = PortIoBus::empty();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, CPUID_EXIT_BUDGET)?;

    let guest_memory = vm
        .guest_memory()
        .expect("registered CPUID memory remains VM-owned");
    let cpuid1_ecx = read_u32(guest_memory, CPUID_GUEST_RESULT)?;
    let kvm_features_eax = read_u32(
        guest_memory,
        GuestPhysAddr::new(CPUID_GUEST_RESULT.get() + 4),
    )?;

    Ok(CpuidGuestResult {
        cpuid1_ecx,
        kvm_features_eax,
        report: execution.report(),
    })
}

pub fn run_long_mode_guest(config: VmConfig) -> Result<LongModeGuestResult, Error> {
    let guest = FlatGuestImage::new(
        LONG_MODE_GUEST_ENTRY,
        LONG_MODE_GUEST_ENTRY,
        &LONG_MODE_GUEST_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(LIFECYCLE_RAM_BASE, LIFECYCLE_RAM_SIZE)?;
    let layout = LongModeBootLayout::new(
        memory.region(),
        LONG_MODE_GUEST_ENTRY,
        LONG_MODE_GUEST_STACK_POINTER,
    )?;
    layout.install_page_tables(&mut memory)?;
    guest.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode(&layout)?;
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, LONG_MODE_EXIT_BUDGET)?;
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != LONG_MODE_GUEST_PROOF {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "long-mode guest proof",
            expected_reason: 5,
            actual_reason: execution.report().exit().reason(),
        }));
    }

    Ok(LongModeGuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
    })
}

fn masked_lapic_features_clear(cpuid1_ecx: u32, kvm_features_eax: u32) -> bool {
    cpuid1_ecx & (CPUID1_X2APIC | CPUID1_TSC_DEADLINE) == 0
        && kvm_features_eax & KVM_FEATURE_PV_UNHALT == 0
}

fn read_u32(memory: &GuestMemory, address: GuestPhysAddr) -> Result<u32, Error> {
    let mut bytes = [0_u8; 4];
    memory.read(address, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_lapic_feature_check_rejects_any_exposed_feature() {
        assert!(masked_lapic_features_clear(0, 0));
        assert!(!masked_lapic_features_clear(CPUID1_X2APIC, 0));
        assert!(!masked_lapic_features_clear(CPUID1_TSC_DEADLINE, 0));
        assert!(!masked_lapic_features_clear(0, KVM_FEATURE_PV_UNHALT));
    }
}
