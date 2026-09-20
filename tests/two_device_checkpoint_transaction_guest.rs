use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::state_snapshot::{
    run_versioned_two_device_checkpoint_transaction_guest, CONTROLLER_CHECKPOINT_PAGE,
    TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR, TWO_VIRTIO_BLK_CHECKPOINT_FIRST_STATUS,
    TWO_VIRTIO_BLK_CHECKPOINT_PROOF, TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR,
    TWO_VIRTIO_BLK_CHECKPOINT_SECOND_STATUS,
};

#[test]
fn two_device_transaction_decodes_both_components_then_executes_restore_and_acceleration() {
    match run_versioned_two_device_checkpoint_transaction_guest() {
        Ok(result) => {
            assert_eq!(result.transaction_version(), 1);
            assert_eq!(result.checkpoint_schema_version(), 1);
            assert_eq!(result.registration_pair_schema_version(), 1);
            assert_eq!(result.registration_pair_encoded_len(), 128);
            assert_eq!(result.registration_versions(), [1, 1]);
            assert_eq!(
                result.encoded_len(),
                48 + result.checkpoint_encoded_len() + result.registration_pair_encoded_len()
            );
            assert_eq!(
                result.bars(),
                [
                    TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR,
                    TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR,
                ]
            );
            assert!(result.canonical_roundtrip());

            let checkpoint = result.checkpoint();
            assert_eq!(
                checkpoint.captured_statuses(),
                [
                    TWO_VIRTIO_BLK_CHECKPOINT_FIRST_STATUS,
                    TWO_VIRTIO_BLK_CHECKPOINT_SECOND_STATUS,
                ]
            );

            let mutation = checkpoint.mutation();
            let mutation_controller = mutation.controller();
            assert_eq!(
                mutation_controller.page_exact(CONTROLLER_CHECKPOINT_PAGE),
                Some(false)
            );
            assert!(!mutation_controller.vcpu_exact());
            assert!(!mutation_controller.master_pic_exact());
            assert!(!mutation_controller.slave_pic_exact());
            assert!(!mutation_controller.ioapic_exact());
            assert!(!mutation_controller.lapic_exact());
            assert_eq!(
                mutation.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR),
                Some(false)
            );
            assert_eq!(
                mutation.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR),
                Some(false)
            );
            assert!(!mutation.is_exact_match());

            let restored = checkpoint.restored();
            let restored_controller = restored.controller();
            assert_eq!(
                restored_controller.page_exact(CONTROLLER_CHECKPOINT_PAGE),
                Some(true)
            );
            assert!(restored_controller.vcpu_exact());
            assert!(restored_controller.master_pic_exact());
            assert!(restored_controller.slave_pic_exact());
            assert!(restored_controller.ioapic_exact());
            assert!(restored_controller.lapic_exact());
            assert_eq!(
                restored.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR),
                Some(true)
            );
            assert_eq!(
                restored.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR),
                Some(true)
            );
            assert!(restored.is_exact_match());
            assert_eq!(
                checkpoint.restored_statuses(),
                checkpoint.captured_statuses()
            );
            assert_eq!(checkpoint.proof(), TWO_VIRTIO_BLK_CHECKPOINT_PROOF);
            assert_eq!(checkpoint.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                checkpoint.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );

            assert_eq!(
                result.acceleration_doorbells(),
                [0x1000_0100, 0x1000_1100]
            );
            assert_eq!(result.acceleration_gsis(), [0, 1]);
            assert_eq!(result.acceleration_vectors(), [0x40, 0x41]);
            assert_eq!(
                result.acceleration_generation_events(),
                [[1, 1], [1, 1]]
            );
            assert_eq!(result.acceleration_proof(), b"RA0MB1NCE0PF1QD");
            assert_eq!(result.acceleration_completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.acceleration_completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping two-device checkpoint transaction assertion: /dev/kvm is unavailable"
            );
        }
        Err(error) => {
            panic!("two-device checkpoint transaction failed unexpectedly: {error}")
        }
    }
}
