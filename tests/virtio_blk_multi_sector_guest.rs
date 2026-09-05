use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::pci::virtio_blk::{
    VIRTIO_BLK_CAPACITY_SECTORS, VIRTIO_BLK_S_OK,
};
use mini_hypervisor::portio::virtio_blk_multi_sector_fixture::{
    deterministic_multi_sector_payload, run_virtio_blk_multi_sector_guest,
    VIRTIO_BLK_MULTI_SECTOR_DATA_LEN, VIRTIO_BLK_MULTI_SECTOR_PROOF,
    VIRTIO_BLK_MULTI_SECTOR_START,
};
use mini_hypervisor::vcpu::VcpuExit;

#[test]
fn guest_round_trips_two_sectors_without_touching_neighbors() {
    match run_virtio_blk_multi_sector_guest(VmConfig::default()) {
        Ok(result) => {
            let expected = deterministic_multi_sector_payload();
            assert_eq!(VIRTIO_BLK_CAPACITY_SECTORS, 4);
            assert_eq!(VIRTIO_BLK_MULTI_SECTOR_START, 1);
            assert_eq!(VIRTIO_BLK_MULTI_SECTOR_DATA_LEN, 1024);

            assert_eq!(result.write_completion().descriptor_id(), 0);
            assert_eq!(result.write_completion().length(), 1);
            assert_eq!(result.write_completion().sector(), 1);
            assert_eq!(result.read_completion().descriptor_id(), 0);
            assert_eq!(result.read_completion().length(), 1025);
            assert_eq!(result.read_completion().sector(), 1);

            assert_eq!(result.request_status(), VIRTIO_BLK_S_OK);
            assert_eq!(result.used_idx(), 2);
            assert_eq!(result.first_used_id(), 0);
            assert_eq!(result.first_used_len(), 1);
            assert_eq!(result.second_used_id(), 0);
            assert_eq!(result.second_used_len(), 1025);
            assert!(result.sector0_unchanged());
            assert!(result.sector3_unchanged());

            assert_eq!(result.backing(), expected);
            assert_eq!(result.readback(), expected);
            assert_eq!(&result.readback()[..16], b"BLK-MULTI-0001!!");
            assert_eq!(&result.readback()[504..512], b"END1BEG2");
            assert_eq!(&result.readback()[512..520], b"-CROSS!!");
            assert_eq!(&result.readback()[1016..], b"MULTEND!");

            assert_eq!(result.proof(), VIRTIO_BLK_MULTI_SECTOR_PROOF);
            assert_eq!(result.io_exits().len(), 21);
            assert_eq!(result.mmio_exits().len(), 22);
            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping multi-sector virtio-blk integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("multi-sector virtio-blk guest failed unexpectedly: {error}"),
    }
}
