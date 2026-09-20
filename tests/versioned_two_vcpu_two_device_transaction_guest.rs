use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::pci::virtio_blk::VIRTIO_BLK_BACKING_SIZE;
use mini_hypervisor::state_snapshot::{
    run_versioned_two_vcpu_two_device_transaction_guest, TWO_VCPU_CHECKPOINT_FIRST_ID,
    TWO_VCPU_CHECKPOINT_FIRST_PROOF, TWO_VCPU_CHECKPOINT_SECOND_ID,
    TWO_VCPU_CHECKPOINT_SECOND_PROOF,
};

const FIRST_BAR: u64 = 0x1000_0000;
const SECOND_BAR: u64 = 0x1000_1000;

#[test]
fn versioned_two_vcpu_two_device_transaction_roundtrips_only_through_bytes_then_restores_runtime_ownership(
) {
    match run_versioned_two_vcpu_two_device_transaction_guest() {
        Ok(result) => {
            assert_eq!(result.schema_version(), 1);
            assert_eq!(result.checkpoint_version(), 1);
            assert_eq!(result.controller_version(), 1);
            assert_eq!(result.registration_pair_version(), 1);
            assert_eq!(result.registration_versions(), [1, 1]);
            assert!(result.canonical_roundtrip());
            assert!(result.encoded_len() > result.checkpoint_encoded_len());
            assert!(result.checkpoint_encoded_len() > result.registration_pair_encoded_len());
            assert_eq!(result.vcpu_ids(), [TWO_VCPU_CHECKPOINT_FIRST_ID, TWO_VCPU_CHECKPOINT_SECOND_ID]);
            assert_eq!(result.mp_states(), [0, 0]);
            assert_eq!(result.page_count(), 3);
            assert_eq!(result.msr_counts(), [0, 0]);
            assert_eq!(result.bars(), [FIRST_BAR, SECOND_BAR]);
            assert_eq!(result.backing_len_each(), VIRTIO_BLK_BACKING_SIZE);

            let transaction = result.transaction();
            assert_eq!(transaction.bars(), [FIRST_BAR, SECOND_BAR]);
            assert_eq!(transaction.captured_statuses(), [0x01, 0x03]);
            assert_eq!(transaction.restored_statuses(), [0x01, 0x03]);
            assert_eq!(transaction.capture_pending(), [false, false]);
            assert_eq!(transaction.reconstructed_pending(), [false, false]);
            assert!(!transaction.mutation().is_exact_match());
            assert!(transaction.restored().is_exact_match());
            assert_eq!(
                transaction
                    .restored()
                    .controller()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                Some(true)
            );
            assert_eq!(
                transaction
                    .restored()
                    .controller()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                Some(true)
            );
            assert_eq!(transaction.first_capture_rip(), 0x1000e);
            assert_eq!(transaction.second_capture_rip(), 0x11006);
            assert_eq!(transaction.first_completion_rip(), 0x10023);
            assert_eq!(transaction.second_completion_rip(), 0x1101b);
            assert_eq!(transaction.first_proof(), TWO_VCPU_CHECKPOINT_FIRST_PROOF);
            assert_eq!(transaction.second_proof(), TWO_VCPU_CHECKPOINT_SECOND_PROOF);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping versioned two-vCPU two-device transaction assertion: /dev/kvm unavailable"
            );
        }
        Err(error) => {
            panic!("versioned two-vCPU two-device transaction failed unexpectedly: {error}")
        }
    }
}
