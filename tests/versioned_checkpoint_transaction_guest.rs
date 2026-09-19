use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::pci::virtio_blk::{deterministic_sector, VIRTIO_BLK_SECTOR_SIZE};
use mini_hypervisor::state_snapshot::{
    run_versioned_checkpoint_transaction_guest, FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE,
    FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF, VERSIONED_CHECKPOINT_TRANSACTION_VERSION,
    VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_VERSION,
};

#[test]
fn outer_transaction_reconstructs_checkpoint_and_host_acceleration_for_mutation_and_replay() {
    match run_versioned_checkpoint_transaction_guest() {
        Ok(result) => {
            assert_eq!(
                result.transaction_version(),
                VERSIONED_CHECKPOINT_TRANSACTION_VERSION
            );
            assert_eq!(
                result.encoded_len(),
                48 + result.checkpoint_encoded_len() + result.registration_encoded_len()
            );
            assert_eq!(
                result.checkpoint_schema_version(),
                VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_VERSION
            );
            assert_eq!(result.registration_schema_version(), 1);
            assert_eq!(result.registration_encoded_len(), 48);
            assert!(result.checkpoint_encoded_len() > VIRTIO_BLK_SECTOR_SIZE);
            assert_eq!(result.page_count(), 1);
            assert!(result.msr_count() > 0);
            assert_eq!(result.bar0(), 0x1000_0000);
            assert_eq!(result.backing_len(), VIRTIO_BLK_SECTOR_SIZE);
            assert!(result.canonical_roundtrip());

            let checkpoint = result.checkpoint();
            assert_eq!(checkpoint.doorbell_gpa(), 0x1000_0100);
            assert_eq!(checkpoint.doorbell_length(), 2);
            assert_eq!(checkpoint.doorbell_datamatch(), 0);
            assert_eq!(checkpoint.gsi(), 0);
            assert_eq!(checkpoint.mutation_doorbell_events(), 1);
            assert_eq!(checkpoint.replay_doorbell_events(), 1);
            assert_eq!(checkpoint.mutation_irqfd_signals(), 1);
            assert_eq!(checkpoint.replay_irqfd_signals(), 1);
            assert_eq!(checkpoint.capture_rflags() & 0x2, 0x2);
            assert_eq!(
                checkpoint.capture_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
            assert_eq!(
                (
                    checkpoint.captured_avail_idx(),
                    checkpoint.captured_used_idx()
                ),
                (0, 0)
            );

            let mutation = checkpoint.mutation();
            let mutation_controller = mutation.controller();
            assert_eq!(
                mutation_controller.page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE),
                Some(false)
            );
            assert!(!mutation_controller.vcpu_exact());
            assert!(!mutation_controller.master_pic_exact());
            assert!(!mutation_controller.slave_pic_exact());
            assert!(!mutation_controller.ioapic_exact());
            assert!(!mutation_controller.lapic_exact());
            assert!(!mutation.device_exact());
            assert!(!mutation.is_exact_match());

            let restored = checkpoint.restored();
            let restored_controller = restored.controller();
            assert_eq!(
                restored_controller.page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE),
                Some(true)
            );
            assert!(restored_controller.vcpu_exact());
            assert!(restored_controller.master_pic_exact());
            assert!(restored_controller.slave_pic_exact());
            assert!(restored_controller.ioapic_exact());
            assert!(restored_controller.lapic_exact());
            assert!(restored.device_exact());
            assert!(restored.is_exact_match());

            assert_eq!(
                checkpoint.mutation_proof(),
                FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF
            );
            assert_eq!(
                checkpoint.replay_proof(),
                FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF
            );
            assert_eq!(
                (checkpoint.replay_avail_idx(), checkpoint.replay_used_idx()),
                (1, 1)
            );
            assert_eq!(checkpoint.replay_rflags() & 0x2, 0x2);
            assert_eq!(
                checkpoint.replay_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
            assert_eq!(checkpoint.backing(), deterministic_sector());
            assert_eq!(checkpoint.readback(), deterministic_sector());
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping versioned checkpoint transaction assertion: /dev/kvm unavailable");
        }
        Err(error) => panic!("versioned checkpoint transaction failed unexpectedly: {error}"),
    }
}
