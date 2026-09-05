use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::pci::virtio_blk::VIRTIO_BLK_S_OK;
use mini_hypervisor::portio::virtio_blk_fixture::{
    deterministic_write_readback_sector, run_virtio_blk_write_readback_guest,
    VIRTIO_BLK_WRITE_READBACK_PROOF,
};
use mini_hypervisor::vcpu::VcpuExit;

#[test]
fn guest_writes_sector_then_reads_same_backing_in_one_vm() {
    match run_virtio_blk_write_readback_guest(VmConfig::default()) {
        Ok(result) => {
            let expected = deterministic_write_readback_sector();
            assert_eq!(result.write_completion().descriptor_id(), 0);
            assert_eq!(result.write_completion().length(), 1);
            assert_eq!(result.write_completion().sector(), 0);
            assert_eq!(result.read_completion().descriptor_id(), 0);
            assert_eq!(result.read_completion().length(), 513);
            assert_eq!(result.read_completion().sector(), 0);
            assert_eq!(result.request_status(), VIRTIO_BLK_S_OK);
            assert_eq!(result.used_idx(), 2);
            assert_eq!(result.first_used_id(), 0);
            assert_eq!(result.first_used_len(), 1);
            assert_eq!(result.second_used_id(), 0);
            assert_eq!(result.second_used_len(), 513);
            assert_eq!(result.backing(), expected);
            assert_eq!(result.readback(), expected);
            assert_eq!(result.proof(), VIRTIO_BLK_WRITE_READBACK_PROOF);
            assert_eq!(result.io_exits().len(), 21);
            assert_eq!(result.mmio_exits().len(), 22);
            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping virtio-blk write/readback integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("virtio-blk write/readback guest failed unexpectedly: {error}"),
    }
}
