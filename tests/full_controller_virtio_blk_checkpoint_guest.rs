use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::pci::virtio_blk::{deterministic_sector, VIRTIO_BLK_SECTOR_SIZE};
use mini_hypervisor::state_snapshot::{
    run_full_controller_virtio_blk_checkpoint_guest, FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE,
    FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF,
};

#[test]
fn full_controller_and_virtio_blk_checkpoint_restore_then_replay_on_kvm() {
    match run_full_controller_virtio_blk_checkpoint_guest() {
        Ok(result) => {
            assert_eq!(result.capture_rflags() & 0x2, 0x2);
            assert_eq!(
                result.capture_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
            assert_eq!(result.captured_avail_idx(), 0);
            assert_eq!(result.captured_used_idx(), 0);

            let mutation = result.mutation();
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
                result.mutation_proof(),
                FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF
            );
            assert_eq!(result.mutation_assert_count(), 1);
            assert_eq!(result.mutation_deassert_count(), 1);

            let restored = result.restored();
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
                result.replay_proof(),
                FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF
            );
            assert_eq!(result.replay_assert_count(), 1);
            assert_eq!(result.replay_deassert_count(), 1);
            assert_eq!(result.replay_avail_idx(), 1);
            assert_eq!(result.replay_used_idx(), 1);
            assert_eq!(result.replay_rflags() & 0x2, 0x2);
            assert_eq!(
                result.replay_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
            assert_eq!(result.backing(), deterministic_sector());
            assert_eq!(result.readback(), deterministic_sector());
            assert_eq!(result.backing().len(), VIRTIO_BLK_SECTOR_SIZE);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping full-controller virtio-blk checkpoint integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!(
            "full-controller virtio-blk checkpoint guest execution failed unexpectedly: {error}"
        ),
    }
}
