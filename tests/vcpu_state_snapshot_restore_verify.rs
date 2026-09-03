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
fn composite_state_snapshot_restores_and_verifies_through_existing_boundaries_when_kvm_is_available() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let vm = backend.create_vm().expect("VM creation should succeed");
    let vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("vCPU creation should succeed");
    let policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty guest MSR policy should be valid");

    vcpu.initialize_real_mode(GuestPhysAddr::new(0x1000))
        .expect("reference real-mode initialization should succeed");
    let reference = vcpu
        .capture_state_snapshot(&policy)
        .expect("reference composite state capture should succeed");

    vcpu.initialize_real_mode(GuestPhysAddr::new(0x1200))
        .expect("changed real-mode initialization should succeed");
    let changed = vcpu
        .capture_state_snapshot(&policy)
        .expect("changed composite state capture should succeed");
    assert!(!reference.compare(&changed).is_exact_match());

    let comparison = vcpu
        .restore_and_verify_state_snapshot(&reference)
        .expect("composite state restore and verification should succeed");

    assert!(comparison.is_exact_match());
    assert_eq!(comparison.registers().reference(), reference.registers());
    assert_eq!(
        comparison.special_registers().reference(),
        reference.special_registers()
    );
    assert_eq!(comparison.msrs().reference(), reference.msrs());
    assert!(comparison.registers().is_exact_match());
    assert!(comparison.special_registers().is_exact_match());
    assert!(comparison.msrs().is_exact_match());
}