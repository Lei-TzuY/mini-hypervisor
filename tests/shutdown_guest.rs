use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::execution::run_vcpu_until_stopped;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::loader::FlatGuestImage;
use mini_hypervisor::memory::{GuestMemory, GuestPhysAddr};
use mini_hypervisor::portio::PortIoBus;
use mini_hypervisor::vcpu::{VcpuExit, VcpuId};

const RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const RAM_SIZE: u64 = 2 * 1024 * 1024;
const ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const KVM_EXIT_SHUTDOWN: u32 = 8;

// LIDT [0x1200] with the zero-filled descriptor already present in guest RAM,
// then INT3. With an IDT limit of zero, exception delivery escalates until KVM
// reports a shutdown/triple-fault exit.
const GUEST_BYTES: [u8; 6] = [0x0f, 0x01, 0x1e, 0x00, 0x12, 0xcc];

fn backend_or_skip() -> Option<KvmBackend> {
    match KvmBackend::open() {
        Ok(backend) => Some(backend),
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping shutdown guest integration assertion: /dev/kvm is unavailable");
            None
        }
        Err(error) => panic!("KVM backend initialization failed unexpectedly: {error}"),
    }
}

#[test]
fn deterministic_triple_fault_is_reported_as_typed_shutdown_when_kvm_is_available() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let image = FlatGuestImage::new(ENTRY, ENTRY, &GUEST_BYTES)
        .expect("shutdown guest image should be valid");
    let mut vm = backend.create_vm().expect("VM creation should succeed");
    let mut memory = GuestMemory::new(RAM_BASE, RAM_SIZE).expect("guest RAM should map");
    image.load(&mut memory).expect("guest image should load");
    vm.register_guest_memory(memory)
        .expect("guest RAM registration should succeed");
    let mut vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("vCPU creation should succeed");
    vcpu.initialize_real_mode(image.entry())
        .expect("real-mode initialization should succeed");
    let mut port_io = PortIoBus::with_debug_port();

    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, 1)
        .expect("triple-fault guest should stop with a shutdown exit");

    assert_eq!(execution.report().vcpu_id(), VcpuId::BOOT);
    assert_eq!(execution.report().exit(), VcpuExit::Shutdown);
    assert_eq!(execution.completed_exits(), 1);
    assert!(execution.io_exits().is_empty());
    assert_eq!(execution.exit_reasons(), [KVM_EXIT_SHUTDOWN]);
    assert_eq!(
        execution.report().exit().reason(),
        execution.exit_reasons()[0]
    );
}
