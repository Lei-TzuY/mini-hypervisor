use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::pci::virtio::VIRTIO_F_VERSION_1;
use mini_hypervisor::portio::pci::virtio_blk::{
    deterministic_sector, VIRTIO_BLK_S_OK,
};
use mini_hypervisor::portio::virtio_blk_fixture::{
    run_virtio_blk_pci_guest, VIRTIO_BLK_PROOF,
};
use mini_hypervisor::vcpu::VcpuExit;

#[test]
fn guest_discovers_and_completes_one_virtio_blk_sector_read() {
    match run_virtio_blk_pci_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.driver_features(), VIRTIO_F_VERSION_1);
            assert!(result.queue_enabled());
            assert_eq!(result.completion().descriptor_id(), 0);
            assert_eq!(result.completion().length(), 513);
            assert_eq!(result.completion().sector(), 0);
            assert_eq!(result.request_status(), VIRTIO_BLK_S_OK);
            assert_eq!(result.used_idx(), 1);
            assert_eq!(result.used_id(), 0);
            assert_eq!(result.used_len(), 513);
            assert_eq!(result.data(), deterministic_sector());
            assert_eq!(result.proof(), VIRTIO_BLK_PROOF);
            assert_eq!(result.io_exits().len(), 18);
            assert_eq!(result.mmio_exits().len(), 21);
            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rip(), result.terminal_rip());
            assert_eq!(result.report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping virtio-blk integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("virtio-blk guest execution failed unexpectedly: {error}"),
    }
}
