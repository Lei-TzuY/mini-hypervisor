use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::kvm::KvmBackend;

#[test]
fn two_fd_free_registration_generations_accelerate_two_independent_sources_on_kvm() {
    match KvmBackend::run_two_host_registration_acceleration_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.doorbells(), [0x1000_0100, 0x1000_1100]);
            assert_eq!(result.gsis(), [0, 1]);
            assert_eq!(result.vectors(), [0x40, 0x41]);
            assert_eq!(result.generation_doorbell_events(), [[1, 1], [1, 1]]);
            assert_eq!(result.proof(), b"RA0MB1NCE0PF1QD");
            assert_eq!(result.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping two-host-registration acceleration assertion: /dev/kvm is unavailable"
            );
        }
        Err(error) => {
            panic!("two-host-registration acceleration failed unexpectedly: {error}")
        }
    }
}
