use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::pci::virtio_blk::{
    deterministic_sector, VIRTIO_BLK_BACKING_SIZE, VIRTIO_BLK_SECTOR_SIZE,
};
use mini_hypervisor::portio::virtio_blk_completion_interrupt_fixture::VIRTIO_BLK_INTERRUPT_BAR0_GPA;
use mini_hypervisor::state_snapshot::{
    run_versioned_full_controller_virtio_blk_checkpoint_guest,
    FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE, FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF,
    VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_VERSION, VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT,
};

#[test]
fn versioned_full_controller_virtio_blk_decodes_materializes_restores_and_replays() {
    match run_versioned_full_controller_virtio_blk_checkpoint_guest() {
        Ok(result) => {
            assert_eq!(
                result.schema_version(),
                VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_VERSION
            );
            assert!(result.encoded_len() > result.backing_len());
            assert_eq!(result.page_count(), 1);
            assert!(result.msr_count() <= VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT);
            assert_eq!(result.bar0(), VIRTIO_BLK_INTERRUPT_BAR0_GPA);
            assert_eq!(result.backing_len(), VIRTIO_BLK_BACKING_SIZE);
            assert!(result.canonical_roundtrip());

            let checkpoint = result.checkpoint();
            assert_eq!(checkpoint.capture_rflags() & 0x2, 0x2);
            assert_eq!(
                checkpoint.capture_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
            assert_eq!(checkpoint.captured_avail_idx(), 0);
            assert_eq!(checkpoint.captured_used_idx(), 0);

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
            assert_eq!(
                checkpoint.mutation_proof(),
                FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF
            );
            assert_eq!(checkpoint.mutation_assert_count(), 1);
            assert_eq!(checkpoint.mutation_deassert_count(), 1);

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
                checkpoint.replay_proof(),
                FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF
            );
            assert_eq!(checkpoint.replay_assert_count(), 1);
            assert_eq!(checkpoint.replay_deassert_count(), 1);
            assert_eq!(checkpoint.replay_avail_idx(), 1);
            assert_eq!(checkpoint.replay_used_idx(), 1);
            assert_eq!(checkpoint.replay_rflags() & 0x2, 0x2);
            assert_eq!(
                checkpoint.replay_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
            assert_eq!(checkpoint.backing(), deterministic_sector());
            assert_eq!(checkpoint.readback(), deterministic_sector());
            assert_eq!(checkpoint.backing().len(), VIRTIO_BLK_SECTOR_SIZE);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping versioned full-controller virtio-blk integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!(
            "versioned full-controller virtio-blk checkpoint guest execution failed unexpectedly: {error}"
        ),
    }
}
