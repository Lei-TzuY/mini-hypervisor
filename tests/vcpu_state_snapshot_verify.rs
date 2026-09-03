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
fn composite_state_snapshot_verification_is_read_only_when_kvm_is_available() {
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

    let exact = vcpu
        .verify_state_snapshot(&reference)
        .expect("exact composite state verification should succeed");
    assert!(exact.is_exact_match());

    vcpu.initialize_real_mode(GuestPhysAddr::new(0x1200))
        .expect("changed real-mode initialization should succeed");
    let changed = vcpu
        .capture_state_snapshot(&policy)
        .expect("changed composite state capture should succeed");
    assert!(!reference.compare(&changed).is_exact_match());

    let mismatch = vcpu
        .verify_state_snapshot(&reference)
        .expect("mismatching composite state verification should still succeed");
    assert!(!mismatch.is_exact_match());
    assert_eq!(mismatch.registers().reference(), reference.registers());
    assert_eq!(
        mismatch.special_registers().reference(),
        reference.special_registers()
    );
    assert_eq!(mismatch.msrs().reference(), reference.msrs());

    let after_verification = vcpu
        .capture_state_snapshot(&policy)
        .expect("post-verification composite state capture should succeed");
    assert!(changed.compare(&after_verification).is_exact_match());
}
