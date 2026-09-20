use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::state_snapshot::{
    run_pending_notification_token_replay_guest, PENDING_NOTIFICATION_TOKEN_PROOF,
};

#[test]
fn pending_notification_token_replays_drained_doorbell_before_queue_service() {
    match run_pending_notification_token_replay_guest() {
        Ok(result) => {
            assert_eq!(result.schema_version(), 3);
            assert_eq!(result.bars(), [0x1000_0000, 0x1000_1000]);
            assert_eq!(result.token_bar(), 0x1000_0000);
            assert_eq!(result.token_queue(), 0);
            assert_eq!(result.token_indices(), [0, 0]);
            assert!(result.ordinary_capture_rejected());
            assert_eq!(result.capture_pending(), [false, false]);
            assert_eq!(result.reconstructed_pending(), [false, false]);
            assert_eq!(result.queue_indices_at_token(), [[0, 0], [0, 0]]);
            assert_eq!(result.queue_indices_after_restore(), [[0, 0], [0, 0]]);
            assert!(result.restored_notification_pending());
            assert!(result.backing_unchanged_at_token());
            assert_eq!(result.final_queue_indices(), [[2, 2], [0, 0]]);
            assert_eq!(result.doorbell_events(), [2, 0]);
            assert_eq!(result.irqfd_signals(), [2, 0]);
            assert_eq!(result.proof(), PENDING_NOTIFICATION_TOKEN_PROOF);
            assert!(result.mutation().device_exact(0x1000_0000) == Some(false));
            assert!(result.restored().is_exact_match());
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping pending-notification token replay assertion: /dev/kvm unavailable"
            );
        }
        Err(error) => panic!("pending-notification token replay failed unexpectedly: {error}"),
    }
}
