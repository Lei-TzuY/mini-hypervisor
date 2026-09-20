use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::kvm::KvmBackend;

#[test]
fn versioned_registration_pair_reconstructs_two_fresh_acceleration_generations_on_kvm() {
    match KvmBackend::run_versioned_two_host_registration_acceleration_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.schema_version(), 1);
            assert_eq!(result.encoded_len(), 128);
            assert_eq!(result.registration_versions(), [1, 1]);
            assert!(result.canonical_roundtrip());

            let acceleration = result.acceleration();
            assert_eq!(acceleration.doorbells(), [0x1000_0100, 0x1000_1100]);
            assert_eq!(acceleration.gsis(), [0, 1]);
            assert_eq!(acceleration.vectors(), [0x40, 0x41]);
            assert_eq!(acceleration.generation_doorbell_events(), [[1, 1], [1, 1]]);
            assert_eq!(acceleration.proof(), b"RA0MB1NCE0PF1QD");
            assert_eq!(acceleration.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                acceleration.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping versioned registration-pair acceleration assertion: /dev/kvm unavailable"
            );
        }
        Err(error) => {
            panic!("versioned registration-pair acceleration failed unexpectedly: {error}")
        }
    }
}
