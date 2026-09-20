use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::state_snapshot::{
    run_restored_multi_producer_data_plane_replay_guest, RESTORED_MULTI_PRODUCER_FIRST_PROOF,
    RESTORED_MULTI_PRODUCER_SECOND_PROOF, TRANSACTION_COUPLED_FIRST_PAGE,
    TRANSACTION_COUPLED_SECOND_PAGE,
};

#[test]
fn restored_versioned_transaction_keeps_each_vcpu_bound_to_its_own_mutable_device_path() {
    match run_restored_multi_producer_data_plane_replay_guest() {
        Ok(result) => {
            assert_eq!(result.schema_version(), 1);
            assert_eq!(result.page_count(), 5);
            assert!(result.canonical_roundtrip());
            assert_eq!(result.bars(), [0x1000_0000, 0x1000_1000]);
            assert_eq!(result.ioapic_entries(), [0x50, 0x0100_0000_0000_0051]);
            assert_eq!(result.capture_pending(), [false, false]);
            assert_eq!(result.reconstructed_pending(), [false, false]);
            assert!(!result.mutation().is_exact_match());
            assert_eq!(
                result
                    .mutation()
                    .controller()
                    .page_exact(TRANSACTION_COUPLED_FIRST_PAGE),
                Some(false)
            );
            assert_eq!(
                result
                    .mutation()
                    .controller()
                    .page_exact(TRANSACTION_COUPLED_SECOND_PAGE),
                Some(false)
            );
            assert!(result.restored().is_exact_match());
            assert_eq!(result.queue_indices(), [[2, 2], [2, 2]]);
            assert_eq!(result.doorbell_events(), [2, 2]);
            assert_eq!(result.irqfd_signals(), [2, 2]);
            assert_eq!(result.readback()[0], result.write_payloads()[0]);
            assert_eq!(result.readback()[1], result.write_payloads()[1]);
            assert_eq!(result.backing()[0], result.write_payloads()[0]);
            assert_eq!(result.backing()[1], result.write_payloads()[1]);
            assert_ne!(result.backing()[0], result.backing()[1]);
            assert_eq!(
                result.producer_proofs()[0],
                RESTORED_MULTI_PRODUCER_FIRST_PROOF
            );
            assert_eq!(
                result.producer_proofs()[1],
                RESTORED_MULTI_PRODUCER_SECOND_PROOF
            );
            assert_eq!(result.capture_rips(), [0x1000e, 0x11006]);
            assert_eq!(result.completion_rips(), [0x10184, 0x1117c]);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping restored multi-producer replay assertion: /dev/kvm unavailable");
        }
        Err(error) => panic!("restored multi-producer replay failed unexpectedly: {error}"),
    }
}
