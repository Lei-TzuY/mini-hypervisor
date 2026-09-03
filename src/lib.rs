#![forbid(unsafe_op_in_unsafe_fn)]

pub mod config;
pub mod error;
pub mod kvm;
pub mod loader;
pub mod memory;
pub mod portio;
pub mod vcpu;
pub mod vmexit;

use config::VmConfig;
use error::{Error, VmExitError};
use kvm::KvmBackend;
use loader::FlatGuestImage;
use memory::{GuestMemory, GuestPhysAddr};
use portio::PortIoBus;
use vcpu::{PortIoExit, VcpuId};
use vmexit::{dispatch_vcpu_exit, VmExitDisposition, VmExitReport};

const LIFECYCLE_RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const LIFECYCLE_RAM_SIZE: u64 = 2 * 1024 * 1024;
const HLT_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const HLT_GUEST_BYTES: [u8; 1] = [0xf4];
const DEBUG_PORT_GUEST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const DEBUG_PORT_GUEST_BYTES: [u8; 5] = [0xb0, b'K', 0xe6, 0xe9, 0xf4];
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
    let exit = vcpu.run_once()?;
    let actual_reason = exit.reason();
    let mut port_io = PortIoBus::empty();

    match dispatch_vcpu_exit(&mut vcpu, exit, &mut port_io)? {
        VmExitDisposition::Stopped(report) => Ok(report),
        VmExitDisposition::Continue(_) => Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "deterministic HLT guest",
            expected_reason: kvm::sys::KVM_EXIT_HLT,
            actual_reason,
        })),
    }
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

    let first_exit = vcpu.run_once()?;
    let first_reason = first_exit.reason();
    let io = match dispatch_vcpu_exit(&mut vcpu, first_exit, &mut port_io)? {
        VmExitDisposition::Continue(io) => io,
        VmExitDisposition::Stopped(_) => {
            return Err(Error::VmExit(VmExitError::UnexpectedSequence {
                stage: "debug-port output",
                expected_reason: kvm::sys::KVM_EXIT_IO,
                actual_reason: first_reason,
            }));
        }
    };

    // KVM documents port-I/O exits as pending until userspace re-enters KVM_RUN. The second run
    // first completes the serviced OUT operation, then continues guest execution to the HLT.
    let second_exit = vcpu.run_once()?;
    let second_reason = second_exit.reason();
    let report = match dispatch_vcpu_exit(&mut vcpu, second_exit, &mut port_io)? {
        VmExitDisposition::Stopped(report) => report,
        VmExitDisposition::Continue(_) => {
            return Err(Error::VmExit(VmExitError::UnexpectedSequence {
                stage: "debug-port termination",
                expected_reason: kvm::sys::KVM_EXIT_HLT,
                actual_reason: second_reason,
            }));
        }
    };

    let output = port_io.debug_output().unwrap_or(&[]).to_vec();
    Ok(DebugPortGuestResult { io, output, report })
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

    let first_exit = vcpu.run_once()?;
    let first_reason = first_exit.reason();
    let io = match dispatch_vcpu_exit(&mut vcpu, first_exit, &mut port_io)? {
        VmExitDisposition::Continue(io) => io,
        VmExitDisposition::Stopped(_) => {
            return Err(Error::VmExit(VmExitError::UnexpectedSequence {
                stage: "debug-port input",
                expected_reason: kvm::sys::KVM_EXIT_IO,
                actual_reason: first_reason,
            }));
        }
    };

    // Re-entry first completes the pending IN by transferring the response byte into AL. Guest
    // code then stores AL at 0x2000 and halts, allowing host RAM inspection to prove consumption.
    let second_exit = vcpu.run_once()?;
    let second_reason = second_exit.reason();
    let report = match dispatch_vcpu_exit(&mut vcpu, second_exit, &mut port_io)? {
        VmExitDisposition::Stopped(report) => report,
        VmExitDisposition::Continue(_) => {
            return Err(Error::VmExit(VmExitError::UnexpectedSequence {
                stage: "debug-port input termination",
                expected_reason: kvm::sys::KVM_EXIT_HLT,
                actual_reason: second_reason,
            }));
        }
    };

    let mut observed = [0_u8; 1];
    vm.guest_memory()
        .expect("registered guest memory remains owned by the VM")
        .read(DEBUG_PORT_INPUT_RESULT, &mut observed)?;

    Ok(DebugPortInputGuestResult {
        io,
        value: observed[0],
        report,
    })
}
