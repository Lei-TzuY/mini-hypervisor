use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::state_snapshot::{
    run_acceleration_checkpoint_quiescence_guest, ACCELERATION_CHECKPOINT_QUIESCENCE_PROOF,
};

#[test]
fn accelerated_checkpoint_rejects_pending_ioeventfd_without_consuming_it_then_captures_after_service(
) {
    match run_acceleration_checkpoint_quiescence_guest() {
        Ok(result) => {
            assert_eq!(result.rejected_pending(), [true, false]);
            assert_eq!(result.pending_after_rejection(), [true, false]);
            assert_eq!(result.preserved_doorbell_count(), 1);
            assert_eq!(result.serviced_queue_indices(), [[1, 1], [0, 0]]);
            assert_eq!(result.captured_queue_indices(), [[1, 1], [0, 0]]);
            assert_eq!(result.proof(), ACCELERATION_CHECKPOINT_QUIESCENCE_PROOF);
            assert_eq!(result.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping acceleration-aware checkpoint quiescence assertion: /dev/kvm unavailable"
            );
        }
        Err(error) => {
            panic!("acceleration-aware checkpoint quiescence failed unexpectedly: {error}")
        }
    }
}
