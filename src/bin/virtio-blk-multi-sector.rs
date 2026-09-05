use mini_hypervisor::config::VmConfig;
use mini_hypervisor::portio::pci::virtio_blk::{
    VIRTIO_BLK_CAPACITY_SECTORS, VIRTIO_BLK_S_OK,
};
use mini_hypervisor::portio::virtio_blk_multi_sector_fixture::{
    deterministic_multi_sector_payload, run_virtio_blk_multi_sector_guest,
    VIRTIO_BLK_MULTI_SECTOR_DATA_LEN, VIRTIO_BLK_MULTI_SECTOR_PROOF,
    VIRTIO_BLK_MULTI_SECTOR_START,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_virtio_blk_multi_sector_guest(VmConfig::default()) {
        Ok(result) => {
            println!(
                "virtio-blk multi-sector range: sector={} length={} capacity={}",
                VIRTIO_BLK_MULTI_SECTOR_START,
                VIRTIO_BLK_MULTI_SECTOR_DATA_LEN,
                VIRTIO_BLK_CAPACITY_SECTORS
            );
            println!(
                "virtio-blk multi-sector write completion: {}/{}/{}",
                result.write_completion().descriptor_id(),
                result.write_completion().length(),
                result.write_completion().sector()
            );
            println!(
                "virtio-blk multi-sector readback completion: {}/{}/{}",
                result.read_completion().descriptor_id(),
                result.read_completion().length(),
                result.read_completion().sector()
            );
            println!(
                "virtio-blk multi-sector used: {}/{}/{}/{}/{}",
                result.used_idx(),
                result.first_used_id(),
                result.first_used_len(),
                result.second_used_id(),
                result.second_used_len()
            );
            println!(
                "virtio-blk multi-sector request status: {}",
                result.request_status()
            );
            println!(
                "virtio-blk multi-sector untouched: sector0={} sector3={}",
                result.sector0_unchanged(),
                result.sector3_unchanged()
            );
            println!(
                "virtio-blk multi-sector backing signatures: first={:?} cross={:?} last={:?}",
                &result.backing()[..16],
                &result.backing()[504..520],
                &result.backing()[1016..]
            );
            println!(
                "virtio-blk multi-sector readback signatures: first={:?} cross={:?} last={:?}",
                &result.readback()[..16],
                &result.readback()[504..520],
                &result.readback()[1016..]
            );
            println!("virtio-blk multi-sector proof: {:?}", result.proof());
            println!(
                "virtio-blk multi-sector port-I/O exits: {}",
                result.io_exits().len()
            );
            println!(
                "virtio-blk multi-sector MMIO exits: {}",
                result.mmio_exits().len()
            );
            println!("{}", result.report());

            let expected = deterministic_multi_sector_payload();
            if VIRTIO_BLK_CAPACITY_SECTORS == 4
                && result.write_completion().descriptor_id() == 0
                && result.write_completion().length() == 1
                && result.write_completion().sector() == VIRTIO_BLK_MULTI_SECTOR_START
                && result.read_completion().descriptor_id() == 0
                && result.read_completion().length() == VIRTIO_BLK_MULTI_SECTOR_DATA_LEN + 1
                && result.read_completion().sector() == VIRTIO_BLK_MULTI_SECTOR_START
                && result.used_idx() == 2
                && result.first_used_id() == 0
                && result.first_used_len() == 1
                && result.second_used_id() == 0
                && result.second_used_len() == VIRTIO_BLK_MULTI_SECTOR_DATA_LEN + 1
                && result.request_status() == VIRTIO_BLK_S_OK
                && result.sector0_unchanged()
                && result.sector3_unchanged()
                && result.backing() == expected
                && result.readback() == expected
                && result.proof() == VIRTIO_BLK_MULTI_SECTOR_PROOF
                && result.io_exits().len() == 21
                && result.mmio_exits().len() == 22
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: multi-sector virtio-blk proof did not match expected state");
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
