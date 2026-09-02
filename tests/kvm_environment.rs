use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::verify_kvm_lifecycle;

#[test]
fn lifecycle_succeeds_when_kvm_is_available() {
    match verify_kvm_lifecycle(VmConfig::default()) {
        Ok(()) => {}
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping KVM integration assertion: /dev/kvm is unavailable to this runner");
        }
        Err(error) => panic!("KVM lifecycle failed unexpectedly: {error}"),
    }
}
