use mini_hypervisor::error::{Error, HostEnvironmentError};
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
fn register_restore_and_verify_returns_owned_exact_comparison_when_kvm_is_available() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let vm = backend.create_vm().expect("VM creation should succeed");
    let vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("vCPU creation should succeed");

    let reference_entry = GuestPhysAddr::new(0x1000);
    vcpu.initialize_real_mode(reference_entry)
        .expect("reference register initialization should succeed");
    let reference = vcpu
        .capture_register_snapshot()
        .expect("reference register snapshot should succeed");

    vcpu.initialize_real_mode(GuestPhysAddr::new(0x1200))
        .expect("changed register initialization should succeed");
    let changed = vcpu
        .capture_register_snapshot()
        .expect("changed register snapshot should succeed");
    assert!(!reference.compare(&changed).is_exact_match());

    let comparison = vcpu
        .restore_and_verify_register_snapshot(&reference)
        .expect("restore-and-verify should return the post-restore comparison");

    assert!(comparison.is_exact_match());
    assert!(comparison.mismatches().is_empty());
    assert_eq!(comparison.reference(), &reference);
    assert_eq!(comparison.observed().rip(), reference_entry.get());
    assert_eq!(comparison.observed().rflags(), reference.rflags());
}
