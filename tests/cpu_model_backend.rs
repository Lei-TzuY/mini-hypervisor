use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::kvm::KvmBackend;

#[test]
fn backend_materializes_owned_cpu_model_candidate_when_kvm_is_available() {
    match KvmBackend::open() {
        Ok(backend) => {
            let candidate = backend.cpu_model_candidate();

            assert_eq!(candidate.guest_cpu_policy(), backend.cpu_policy());
            assert_eq!(
                candidate.host_msr_model_candidate().source_observation(),
                backend.host_msr_feature_values()
            );

            let comparison = candidate.compare(&candidate);
            drop(candidate);
            drop(backend);

            assert!(comparison.is_exact_match());
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping CPU model backend integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("KVM backend discovery failed unexpectedly: {error}"),
    }
}
