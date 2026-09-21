use mini_hypervisor::portio::pci::virtio_blk::{
    run_file_backed_reopen_proof, FILE_BACKED_VIRTIO_BLK_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_file_backed_reopen_proof() {
        Ok(result) => {
            println!(
                "file-backed write completion: id={} len={} sector={}",
                result.write_completion().descriptor_id(),
                result.write_completion().length(),
                result.write_completion().sector()
            );
            println!(
                "file-backed read completion: id={} len={} sector={}",
                result.read_completion().descriptor_id(),
                result.read_completion().length(),
                result.read_completion().sector()
            );
            println!(
                "file-backed persisted/readback exact: {}",
                result.persisted_sector() == result.readback()
            );
            println!(
                "file-backed checkpoint rejected: {}",
                result.checkpoint_rejected()
            );
            println!("file-backed proof: {:?}", result.proof());
            if result.proof() != FILE_BACKED_VIRTIO_BLK_PROOF {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
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
