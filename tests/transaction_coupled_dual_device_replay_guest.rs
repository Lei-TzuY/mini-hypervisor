use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::state_snapshot::{
    run_transaction_coupled_dual_device_replay_guest, TRANSACTION_COUPLED_REPLAY_PROOF,
};

#[test]
fn decoded_transaction_replays_two_devices_with_fresh_acceleration_on_the_restored_vm() {
    match run_transaction_coupled_dual_device_replay_guest() {
        Ok(result) => {
            assert_eq!(result.transaction_version(), 1);
            assert_eq!(result.bars(), [0x1000_0000, 0x1000_1000]);
            assert!(!result.mutation().is_exact_match());
            assert_eq!(
                result.mutation().device_exact(result.bars()[0]),
                Some(false)
            );
            assert_eq!(
                result.mutation().device_exact(result.bars()[1]),
                Some(false)
            );
            assert!(result.restored().is_exact_match());
            assert_eq!(result.replay_queue_indices(), [[1, 1], [1, 1]]);
            assert_eq!(result.doorbell_events(), [1, 1]);
            assert_eq!(result.irqfd_signals(), [1, 1]);
            assert_eq!(result.readback()[0], result.backing()[0]);
            assert_eq!(result.readback()[1], result.backing()[1]);
            assert_eq!(result.proof(), TRANSACTION_COUPLED_REPLAY_PROOF);
            assert_eq!(result.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping coupled dual-device replay assertion: /dev/kvm unavailable");
        }
        Err(error) => panic!("coupled dual-device replay failed unexpectedly: {error}"),
    }
}
