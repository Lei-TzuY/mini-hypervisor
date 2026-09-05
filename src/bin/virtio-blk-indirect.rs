use mini_hypervisor::config::VmConfig;
use mini_hypervisor::portio::pci::virtio::VIRTIO_F_VERSION_1;
use mini_hypervisor::portio::pci::virtio_blk::{VIRTIO_BLK_S_OK, VIRTIO_RING_F_INDIRECT_DESC};
use mini_hypervisor::portio::virtio_blk_fixture::{
    deterministic_write_readback_sector, run_virtio_blk_indirect_guest, VIRTIO_BLK_INDIRECT_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_virtio_blk_indirect_guest(VmConfig::default()) {
        Ok(result) => {
            println!(
                "virtio-blk indirect driver features: {:#x}",
                result.driver_features()
            );
            println!(
                "virtio-blk indirect write completion: {}/{}/{}",
                result.write_completion().descriptor_id(),
                result.write_completion().length(),
                result.write_completion().sector()
            );
            println!(
                "virtio-blk indirect readback completion: {}/{}/{}",
                result.read_completion().descriptor_id(),
                result.read_completion().length(),
                result.read_completion().sector()
            );
            println!(
                "virtio-blk indirect used: {}/{}/{}/{}/{}",
                result.used_idx(),
                result.first_used_id(),
                result.first_used_len(),
                result.second_used_id(),
                result.second_used_len()
            );
            println!(
                "virtio-blk indirect request status: {}",
                result.request_status()
            );
            println!("virtio-blk indirect proof: {:?}", result.proof());
            println!(
                "virtio-blk indirect port-I/O exits: {}",
                result.io_exits().len()
            );
            println!(
                "virtio-blk indirect MMIO exits: {}",
                result.mmio_exits().len()
            );
            println!("{}", result.report());

            let expected = deterministic_write_readback_sector();
            let expected_features = VIRTIO_F_VERSION_1 | VIRTIO_RING_F_INDIRECT_DESC;
            if result.driver_features() == expected_features
                && result.write_completion().descriptor_id() == 0
                && result.write_completion().length() == 1
                && result.write_completion().sector() == 0
                && result.read_completion().descriptor_id() == 0
                && result.read_completion().length() == 513
                && result.read_completion().sector() == 0
                && result.used_idx() == 2
                && result.first_used_id() == 0
                && result.first_used_len() == 1
                && result.second_used_id() == 0
                && result.second_used_len() == 513
                && result.request_status() == VIRTIO_BLK_S_OK
                && result.backing() == expected
                && result.readback() == expected
                && result.proof() == VIRTIO_BLK_INDIRECT_PROOF
                && result.io_exits().len() == 22
                && result.mmio_exits().len() == 26
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: virtio-blk indirect proof did not match expected state");
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("error: {error}");
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                eprintln!("caused by: {cause}");
                source = cause.source();
            }
            ExitCode::FAILURE
        }
    }
}
