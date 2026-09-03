use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::execution::run_vcpu_until_stopped;
use mini_hypervisor::kvm::{sys, KvmBackend};
use mini_hypervisor::loader::FlatGuestImage;
use mini_hypervisor::memory::{GuestMemory, GuestPhysAddr};
use mini_hypervisor::portio::PortIoBus;
use mini_hypervisor::vcpu::VcpuId;

const RAM_BASE: GuestPhysAddr = GuestPhysAddr::new(0);
const RAM_SIZE: u64 = 2 * 1024 * 1024;
const ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1000);
const GUEST_BYTES: [u8; 5] = [0xb0, b'K', 0xe6, 0xe9, 0xf4];

fn backend_or_skip() -> Option<KvmBackend> {
    match KvmBackend::open() {
        Ok(backend) => Some(backend),
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping KVM integration assertion: /dev/kvm is unavailable to this runner");
            None
        }
        Err(error) => panic!("KVM backend initialization failed unexpectedly: {error}"),
    }
}

#[test]
fn successful_execution_preserves_each_completed_exit_reason_in_order_when_kvm_is_available() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let image = FlatGuestImage::new(ENTRY, ENTRY, &GUEST_BYTES)
        .expect("debug-port guest image should be valid");
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

    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, 2)
        .expect("debug-port guest should reach HLT");

    assert_eq!(
        execution.exit_reasons(),
        &[sys::KVM_EXIT_IO, sys::KVM_EXIT_HLT]
    );
    assert_eq!(
        execution.exit_reasons().len(),
        execution.completed_exits() as usize
    );
    assert_eq!(
        execution.exit_reasons().last().copied(),
        Some(execution.report().exit().reason())
    );
}
