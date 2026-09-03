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
fn special_register_snapshot_observes_and_restores_real_mode_state_when_kvm_is_available() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let vm = backend.create_vm().expect("VM creation should succeed");
    let vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("vCPU creation should succeed");

    vcpu.initialize_real_mode(GuestPhysAddr::new(0x1000))
        .expect("real-mode initialization should succeed");
    let snapshot = vcpu
        .capture_special_register_snapshot()
        .expect("special-register capture should succeed");

    for segment in [
        snapshot.cs(),
        snapshot.ds(),
        snapshot.es(),
        snapshot.fs(),
        snapshot.gs(),
        snapshot.ss(),
    ] {
        assert_eq!(segment.base(), 0);
        assert_eq!(segment.selector(), 0);
    }

    assert_eq!(snapshot.cr0() & 1, 0, "CR0.PE should be cleared");
    assert_eq!(snapshot.cr0() & (1 << 31), 0, "CR0.PG should be cleared");

    vcpu.restore_special_register_snapshot(&snapshot)
        .expect("captured special-register snapshot should restore successfully");
    let restored = vcpu
        .capture_special_register_snapshot()
        .expect("restored special-register state should be observable");
    let restore_comparison = snapshot.compare(&restored);
    assert!(restore_comparison.is_exact_match());
    assert!(restore_comparison.mismatches().is_empty());

    let copied = snapshot;
    drop(vcpu);
    assert_eq!(copied.cr0(), snapshot.cr0());
    assert_eq!(copied.interrupt_bitmap(), snapshot.interrupt_bitmap());

    let comparison = snapshot.compare(&copied);
    assert!(comparison.is_exact_match());
    assert!(comparison.mismatches().is_empty());
    assert_eq!(comparison.reference(), &snapshot);
    assert_eq!(comparison.observed(), &copied);
}
