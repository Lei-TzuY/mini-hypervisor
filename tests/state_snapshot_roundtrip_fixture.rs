use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::run_state_snapshot_roundtrip;

#[test]
fn public_state_snapshot_roundtrip_reports_changed_then_exact_restored_state_when_kvm_is_available() {
    let result = match run_state_snapshot_roundtrip(VmConfig::default()) {
        Ok(result) => result,
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping KVM integration assertion: /dev/kvm is unavailable to this runner");
            return;
        }
        Err(error) => panic!("state snapshot round-trip failed unexpectedly: {error}"),
    };

    assert!(!result.changed().is_exact_match());
    assert!(result.restored().is_exact_match());
    assert!(result.restored().registers().is_exact_match());
    assert!(result.restored().special_registers().is_exact_match());
    assert!(result.restored().msrs().is_exact_match());
}
