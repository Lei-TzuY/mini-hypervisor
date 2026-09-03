use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::kvm::msr::GuestMsrAccessPolicy;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::memory::GuestPhysAddr;
use mini_hypervisor::vcpu::VcpuId;

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
fn composite_vcpu_state_snapshot_owns_existing_state_components_when_kvm_is_available() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let vm = backend.create_vm().expect("VM creation should succeed");
    let vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("vCPU creation should succeed");

    vcpu.initialize_real_mode(GuestPhysAddr::new(0x1000))
        .expect("real-mode initialization should succeed");
    let policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty guest MSR policy should be valid");

    let snapshot = vcpu
        .capture_state_snapshot(&policy)
        .expect("composite vCPU state capture should succeed");

    assert_eq!(snapshot.registers().rip(), 0x1000);
    assert_eq!(snapshot.special_registers().cr0() & 1, 0);
    assert!(snapshot.msrs().policy().entries().is_empty());
    assert!(snapshot.msrs().values().values().is_empty());

    drop(policy);
    drop(vcpu);

    assert_eq!(snapshot.registers().rip(), 0x1000);
    assert!(snapshot.msrs().policy().entries().is_empty());
}
