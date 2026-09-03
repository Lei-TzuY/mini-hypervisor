use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::kvm::msr::GuestMsrAccessPolicy;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::memory::GuestPhysAddr;
use mini_hypervisor::vcpu::{VcpuId, VcpuRegisterField};

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
fn composite_state_comparison_delegates_to_existing_component_comparisons_when_kvm_is_available() {
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

    let identical = vcpu
        .capture_state_snapshot(&policy)
        .expect("identical composite state capture should succeed");
    let exact = reference.compare(&identical);
    assert!(exact.is_exact_match());
    assert!(exact.registers().is_exact_match());
    assert!(exact.special_registers().is_exact_match());
    assert!(exact.msrs().is_exact_match());

    vcpu.initialize_real_mode(GuestPhysAddr::new(0x1001))
        .expect("observed real-mode initialization should succeed");
    let observed = vcpu
        .capture_state_snapshot(&policy)
        .expect("observed composite state capture should succeed");
    let comparison = reference.compare(&observed);

    assert!(!comparison.is_exact_match());
    assert_eq!(comparison.registers().mismatches().len(), 1);
    assert_eq!(
        comparison.registers().mismatches()[0].field(),
        VcpuRegisterField::Rip
    );
    assert!(comparison.special_registers().is_exact_match());
    assert!(comparison.msrs().is_exact_match());
}
