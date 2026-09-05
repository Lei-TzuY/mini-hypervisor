use mini_hypervisor::config::VmConfig;
use mini_hypervisor::portio::pci::virtio_blk::VIRTIO_BLK_S_OK;
use mini_hypervisor::portio::virtio_blk_fixture::{
    deterministic_write_readback_sector, run_virtio_blk_write_readback_guest,
    VIRTIO_BLK_WRITE_READBACK_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_virtio_blk_write_readback_guest(VmConfig::default()) {
        Ok(result) => {
            println!(
                "virtio-blk write completion: {}/{}/{}",
                result.write_completion().descriptor_id(),
                result.write_completion().length(),
                result.write_completion().sector()
            );
            println!(
                "virtio-blk readback completion: {}/{}/{}",
                result.read_completion().descriptor_id(),
                result.read_completion().length(),
                result.read_completion().sector()
            );
            println!(
                "virtio-blk write/readback used: {}/{}/{}/{}/{}",
                result.used_idx(),
                result.first_used_id(),
                result.first_used_len(),
                result.second_used_id(),
                result.second_used_len()
            );
            println!(
                "virtio-blk write/readback request status: {}",
                result.request_status()
            );
            println!(
                "virtio-blk backing boundary: first={:?} last={:?}",
                &result.backing()[..16],
                &result.backing()[result.backing().len() - 8..]
            );
            println!(
                "virtio-blk readback boundary: first={:?} last={:?}",
                &result.readback()[..16],
                &result.readback()[result.readback().len() - 8..]
            );
            println!("virtio-blk write/readback proof: {:?}", result.proof());
            println!(
                "virtio-blk write/readback port-I/O exits: {}",
                result.io_exits().len()
            );
            println!(
                "virtio-blk write/readback MMIO exits: {}",
                result.mmio_exits().len()
            );
            println!("{}", result.report());

            let expected = deterministic_write_readback_sector();
            if result.write_completion().descriptor_id() == 0
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
                && result.proof() == VIRTIO_BLK_WRITE_READBACK_PROOF
                && result.io_exits().len() == 21
                && result.mmio_exits().len() == 22
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: virtio-blk write/readback proof did not match expected state");
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
