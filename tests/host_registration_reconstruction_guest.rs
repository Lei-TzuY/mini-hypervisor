use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::pci::virtio_blk::{deterministic_sector, VIRTIO_BLK_SECTOR_SIZE};
use mini_hypervisor::state_snapshot::{
    run_full_controller_virtio_blk_host_registration_reconstruction_guest,
    FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE, FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF,
};

#[test]
fn full_controller_virtio_blk_restore_reconstructs_fresh_host_registrations() {
    match run_full_controller_virtio_blk_host_registration_reconstruction_guest() {
        Ok(result) => {
            assert_eq!(result.doorbell_gpa(), 0x1000_0100);
            assert_eq!(result.doorbell_length(), 2);
            assert_eq!(result.doorbell_datamatch(), 0);
            assert_eq!(result.gsi(), 0);
            assert_eq!(result.mutation_doorbell_events(), 1);
            assert_eq!(result.replay_doorbell_events(), 1);
            assert_eq!(result.mutation_irqfd_signals(), 1);
            assert_eq!(result.replay_irqfd_signals(), 1);

            assert_eq!(result.capture_rflags() & 0x2, 0x2);
            assert_eq!(result.capture_rflags() & X86_RFLAGS_INTERRUPT_ENABLE, X86_RFLAGS_INTERRUPT_ENABLE);
            assert_eq!((result.captured_avail_idx(), result.captured_used_idx()), (0, 0));

            let mutation = result.mutation();
            let mc = mutation.controller();
            assert_eq!(mc.page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE), Some(false));
            assert!(!mc.vcpu_exact());
            assert!(!mc.master_pic_exact());
            assert!(!mc.slave_pic_exact());
            assert!(!mc.ioapic_exact());
            assert!(!mc.lapic_exact());
            assert!(!mutation.device_exact());

            let restored = result.restored();
            let rc = restored.controller();
            assert_eq!(rc.page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE), Some(true));
            assert!(rc.vcpu_exact());
            assert!(rc.master_pic_exact());
            assert!(rc.slave_pic_exact());
            assert!(rc.ioapic_exact());
            assert!(rc.lapic_exact());
            assert!(restored.device_exact());
            assert!(restored.is_exact_match());

            assert_eq!(result.mutation_proof(), FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF);
            assert_eq!(result.replay_proof(), FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF);
            assert_eq!((result.replay_avail_idx(), result.replay_used_idx()), (1, 1));
            assert_eq!(result.replay_rflags() & 0x2, 0x2);
            assert_eq!(result.replay_rflags() & X86_RFLAGS_INTERRUPT_ENABLE, X86_RFLAGS_INTERRUPT_ENABLE);
            assert_eq!(result.backing(), deterministic_sector());
            assert_eq!(result.readback(), deterministic_sector());
            assert_eq!(result.backing().len(), VIRTIO_BLK_SECTOR_SIZE);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping host-registration reconstruction assertion: /dev/kvm unavailable");
        }
        Err(error) => panic!("host-registration reconstruction failed unexpectedly: {error}"),
    }
}
