#![forbid(unsafe_op_in_unsafe_fn)]

pub mod config;
pub mod error;
pub mod execution;
pub mod kvm;
pub mod loader;
pub mod memory;
pub mod portio;
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
