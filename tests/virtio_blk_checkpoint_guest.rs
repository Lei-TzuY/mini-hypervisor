use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::state_snapshot::{
    run_virtio_blk_checkpoint_guest, VIRTIO_BLK_CHECKPOINT_CAPTURE_PROOF,
    VIRTIO_BLK_CHECKPOINT_PAGE, VIRTIO_BLK_CHECKPOINT_REPLAY_PROOF,
};
use mini_hypervisor::vcpu::VcpuExit;

#[test]
fn virtio_blk_checkpoint_restores_guest_and_device_then_replays_request() {
    match run_virtio_blk_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.capture_report().exit(), VcpuExit::Hlt);
            assert_eq!(result.capture_report().rflags() & 0x2, 0x2);
            assert_eq!(result.capture_proof(), VIRTIO_BLK_CHECKPOINT_CAPTURE_PROOF);
            assert_eq!(result.captured_avail_idx(), 1);
            assert_eq!(result.captured_used_idx(), 1);

            assert_eq!(result.mutation_report().exit(), VcpuExit::Hlt);
            assert_eq!(result.mutation_report().rflags() & 0x2, 0x2);
            assert_eq!(result.mutation_proof(), VIRTIO_BLK_CHECKPOINT_REPLAY_PROOF);
            assert_eq!(
                result.mutation().page_exact(VIRTIO_BLK_CHECKPOINT_PAGE),
                Some(false)
            );
            assert!(!result.mutation().vcpu_exact());
            assert!(!result.mutation().device_exact());
            assert!(!result.mutation().is_exact_match());

            assert_eq!(
                result.restored().page_exact(VIRTIO_BLK_CHECKPOINT_PAGE),
                Some(true)
            );
            assert!(result.restored().vcpu_exact());
            assert!(result.restored().device_exact());
            assert!(result.restored().is_exact_match());

            assert_eq!(result.replay_report().exit(), VcpuExit::Hlt);
            assert_eq!(result.replay_report().rflags() & 0x2, 0x2);
            assert_eq!(result.replay_proof(), VIRTIO_BLK_CHECKPOINT_REPLAY_PROOF);
            assert_eq!(result.replay_avail_idx(), 2);
            assert_eq!(result.replay_used_idx(), 2);
            assert_eq!(result.mutation_report().rip(), result.replay_report().rip());
            assert!(result.capture_report().rip() < result.replay_report().rip());
            assert_eq!(result.backing(), result.readback());
            assert_eq!(result.backing().len(), 512);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping virtio-blk checkpoint integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => {
            panic!("virtio-blk checkpoint guest execution failed unexpectedly: {error}")
        }
    }
}
