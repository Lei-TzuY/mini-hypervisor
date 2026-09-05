use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::pci::virtio::VIRTIO_F_VERSION_1;
use mini_hypervisor::portio::pci::virtio_blk::{VIRTIO_BLK_S_OK, VIRTIO_RING_F_INDIRECT_DESC};
use mini_hypervisor::portio::virtio_blk_fixture::{
    deterministic_write_readback_sector, run_virtio_blk_indirect_guest, VIRTIO_BLK_INDIRECT_PROOF,
};
use mini_hypervisor::vcpu::{MmioDirection, VcpuExit};

#[test]
fn guest_negotiates_indirect_feature_and_round_trips_through_indirect_tables() {
    match run_virtio_blk_indirect_guest(VmConfig::default()) {
        Ok(result) => {
            let expected = deterministic_write_readback_sector();
            assert_eq!(
                result.driver_features(),
                VIRTIO_F_VERSION_1 | VIRTIO_RING_F_INDIRECT_DESC
            );
            assert_eq!(result.write_completion().descriptor_id(), 0);
            assert_eq!(result.write_completion().length(), 1);
            assert_eq!(result.read_completion().descriptor_id(), 0);
            assert_eq!(result.read_completion().length(), 513);
            assert_eq!(result.used_idx(), 2);
            assert_eq!(result.first_used_id(), 0);
            assert_eq!(result.second_used_id(), 0);
            assert_eq!(result.request_status(), VIRTIO_BLK_S_OK);
            assert_eq!(result.backing(), expected);
            assert_eq!(result.readback(), expected);
            assert_eq!(result.proof(), VIRTIO_BLK_INDIRECT_PROOF);
            assert_eq!(result.io_exits().len(), 22);
            assert_eq!(result.mmio_exits().len(), 26);

            let low_select = &result.mmio_exits()[3];
            let low_read = &result.mmio_exits()[4];
            let low_driver_select = &result.mmio_exits()[5];
            let low_driver_write = &result.mmio_exits()[6];
            assert_eq!(low_select.direction(), MmioDirection::Write);
            assert_eq!(low_select.write_data(), &0_u32.to_le_bytes());
            assert_eq!(low_read.direction(), MmioDirection::Read);
            assert_eq!(low_driver_select.write_data(), &0_u32.to_le_bytes());
            assert_eq!(
                low_driver_write.write_data(),
                &(VIRTIO_RING_F_INDIRECT_DESC as u32).to_le_bytes()
            );
            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping virtio-blk indirect integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("virtio-blk indirect guest failed unexpectedly: {error}"),
    }
}
