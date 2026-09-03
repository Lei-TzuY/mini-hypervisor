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
fn component_snapshot_verification_is_read_only_when_kvm_is_available() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let vm = backend.create_vm().expect("VM creation should succeed");
    let vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("vCPU creation should succeed");
    let policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty guest MSR policy should be valid");

    let pre_init_special = vcpu
        .capture_special_register_snapshot()
        .expect("pre-initialization special-register capture should succeed");
    let special_exact = vcpu
        .verify_special_register_snapshot(&pre_init_special)
        .expect("exact special-register verification should succeed");
    assert!(special_exact.is_exact_match());

    vcpu.initialize_real_mode(GuestPhysAddr::new(0x1000))
        .expect("reference real-mode initialization should succeed");

    let real_mode_special = vcpu
        .capture_special_register_snapshot()
        .expect("real-mode special-register capture should succeed");
    assert!(!pre_init_special
        .compare(&real_mode_special)
        .is_exact_match());
    let special_mismatch = vcpu
        .verify_special_register_snapshot(&pre_init_special)
        .expect("mismatching special-register verification should still succeed");
    assert!(!special_mismatch.is_exact_match());
    let special_after_verify = vcpu
        .capture_special_register_snapshot()
        .expect("post-verification special-register capture should succeed");
    assert!(real_mode_special
        .compare(&special_after_verify)
        .is_exact_match());

    let register_reference = vcpu
        .capture_register_snapshot()
        .expect("reference register capture should succeed");
    let register_exact = vcpu
        .verify_register_snapshot(&register_reference)
        .expect("exact register verification should succeed");
    assert!(register_exact.is_exact_match());

    vcpu.initialize_real_mode(GuestPhysAddr::new(0x1200))
        .expect("changed real-mode initialization should succeed");
    let changed_registers = vcpu
        .capture_register_snapshot()
        .expect("changed register capture should succeed");
    assert!(!register_reference
        .compare(&changed_registers)
        .is_exact_match());
    let register_mismatch = vcpu
        .verify_register_snapshot(&register_reference)
        .expect("mismatching register verification should still succeed");
    assert!(!register_mismatch.is_exact_match());
    let registers_after_verify = vcpu
        .capture_register_snapshot()
        .expect("post-verification register capture should succeed");
    assert!(changed_registers
        .compare(&registers_after_verify)
        .is_exact_match());

    let msr_reference = vcpu
        .capture_msr_snapshot(&policy)
        .expect("reference MSR capture should succeed");
    let msrs_before_verify = vcpu
        .capture_msr_snapshot(&policy)
        .expect("pre-verification MSR capture should succeed");
    let msr_exact = vcpu
        .verify_msr_snapshot(&msr_reference)
        .expect("MSR verification should succeed");
    assert!(msr_exact.is_exact_match());
    assert_eq!(msr_exact.reference(), &msr_reference);
    let msrs_after_verify = vcpu
        .capture_msr_snapshot(&policy)
        .expect("post-verification MSR capture should succeed");
    assert!(msrs_before_verify
        .compare(&msrs_after_verify)
        .is_exact_match());
}
