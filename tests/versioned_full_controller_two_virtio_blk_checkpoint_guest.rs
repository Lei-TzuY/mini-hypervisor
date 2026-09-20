use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::pci::virtio_blk::VIRTIO_BLK_BACKING_SIZE;
use mini_hypervisor::state_snapshot::{
    run_versioned_full_controller_two_virtio_blk_checkpoint_guest, CONTROLLER_CHECKPOINT_PAGE,
    TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR, TWO_VIRTIO_BLK_CHECKPOINT_FIRST_STATUS,
    TWO_VIRTIO_BLK_CHECKPOINT_PROOF, TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR,
    TWO_VIRTIO_BLK_CHECKPOINT_SECOND_STATUS, VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_VERSION,
};

#[test]
fn versioned_two_device_checkpoint_crosses_bytes_then_restores_exactly_on_kvm() {
    match run_versioned_full_controller_two_virtio_blk_checkpoint_guest() {
        Ok(result) => {
            assert_eq!(
                result.schema_version(),
                VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_VERSION
            );
            assert!(result.encoded_len() > 2 * VIRTIO_BLK_BACKING_SIZE);
            assert_eq!(result.page_count(), 1);
            assert_eq!(result.msr_count(), 0);
            assert_eq!(
                result.bars(),
                [
                    TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR,
                    TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR,
                ]
            );
            assert_eq!(result.backing_len_each(), VIRTIO_BLK_BACKING_SIZE);
            assert!(result.canonical_roundtrip());

            let checkpoint = result.checkpoint();
            assert_eq!(checkpoint.captured_bars(), result.bars());
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
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping versioned two-virtio-blk checkpoint assertion: /dev/kvm is unavailable"
            );
        }
        Err(error) => {
            panic!("versioned two-virtio-blk checkpoint execution failed unexpectedly: {error}")
        }
    }
}
