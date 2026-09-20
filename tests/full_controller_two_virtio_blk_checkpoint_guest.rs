use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::state_snapshot::{
    run_full_controller_two_virtio_blk_checkpoint_guest, CONTROLLER_CHECKPOINT_PAGE,
    TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR, TWO_VIRTIO_BLK_CHECKPOINT_FIRST_STATUS,
    TWO_VIRTIO_BLK_CHECKPOINT_PROOF, TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR,
    TWO_VIRTIO_BLK_CHECKPOINT_SECOND_STATUS,
};

#[test]
fn full_controller_restores_two_distinct_quiescent_virtio_blk_devices_on_kvm() {
    match run_full_controller_two_virtio_blk_checkpoint_guest() {
        Ok(result) => {
            assert_eq!(
                result.captured_bars(),
                [
                    TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR,
                    TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR,
                ]
            );
            assert_eq!(
                result.captured_statuses(),
                [
                    TWO_VIRTIO_BLK_CHECKPOINT_FIRST_STATUS,
                    TWO_VIRTIO_BLK_CHECKPOINT_SECOND_STATUS,
                ]
            );

            let mutation = result.mutation();
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

            let restored = result.restored();
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
            assert_eq!(result.restored_statuses(), result.captured_statuses());
            assert_eq!(result.proof(), TWO_VIRTIO_BLK_CHECKPOINT_PROOF);
            assert_eq!(result.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping two-virtio-blk full-controller checkpoint integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!(
            "two-virtio-blk full-controller checkpoint execution failed unexpectedly: {error}"
        ),
    }
}
