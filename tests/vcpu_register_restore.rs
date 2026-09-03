use mini_hypervisor::error::{Error, HostEnvironmentError};
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
fn vcpu_register_snapshot_restore_round_trips_owned_general_register_state_when_kvm_is_available() {
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

    let changed_entry = GuestPhysAddr::new(0x1200);
    vcpu.initialize_real_mode(changed_entry)
        .expect("changed register initialization should succeed");
    let changed = vcpu
        .capture_register_snapshot()
        .expect("changed register snapshot should succeed");
    let changed_comparison = reference.compare(&changed);
    assert!(!changed_comparison.is_exact_match());
    assert!(changed_comparison
        .mismatches()
        .iter()
        .any(|mismatch| mismatch.field() == VcpuRegisterField::Rip));

    vcpu.restore_register_snapshot(&reference)
        .expect("restoring the owned register snapshot should succeed");
    let restored = vcpu
        .capture_register_snapshot()
        .expect("restored register snapshot should succeed");

    let restored_comparison = reference.compare(&restored);
    assert!(restored_comparison.is_exact_match());
    assert!(restored_comparison.mismatches().is_empty());
    assert_eq!(restored.rip(), reference_entry.get());
    assert_eq!(restored.rflags(), reference.rflags());
}
