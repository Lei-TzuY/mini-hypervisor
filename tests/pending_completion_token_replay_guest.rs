use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR;
use mini_hypervisor::state_snapshot::{
    run_pending_completion_token_replay_guest, PENDING_COMPLETION_TOKEN_PROOF,
};

#[test]
fn serviced_completion_crosses_v2_checkpoint_before_fresh_irqfd_delivery() {
    match run_pending_completion_token_replay_guest() {
        Ok(result) => {
            assert_eq!(result.schema_version(), 2);
            assert_eq!(result.page_count(), 5);
            assert_eq!(result.token_bar(), TWO_HOST_REGISTRATION_FIRST_BAR);
            assert_eq!(result.token_queue(), 0);
            assert_eq!(result.token_indices(), [1, 1]);
            assert!(result.ordinary_capture_rejected());
            assert_eq!(result.capture_pending(), [false, false]);
            assert_eq!(result.reconstructed_pending(), [false, false]);
            assert_eq!(result.queue_indices_at_token(), [[1, 1], [0, 0]]);
            assert_eq!(result.queue_indices_after_restore(), [[1, 1], [0, 0]]);
            assert_eq!(result.final_queue_indices(), [[2, 2], [0, 0]]);
            assert_eq!(result.doorbell_events(), [2, 0]);
            assert_eq!(result.irqfd_signals(), [2, 0]);
            assert!(result.restored().is_exact_match());
            assert_eq!(result.proof(), PENDING_COMPLETION_TOKEN_PROOF);
            assert!(result.pending_rip() > result.capture_rips()[0]);
            assert!(result.completion_rip() > result.pending_rip());
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping pending-completion token replay assertion: /dev/kvm unavailable");
        }
        Err(error) => panic!("pending-completion token replay failed unexpectedly: {error}"),
    }
}
