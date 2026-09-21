use mini_hypervisor::portio::pci::virtio_blk::{
    run_file_backed_identity_pin_proof, FILE_BACKED_IDENTITY_PIN_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_file_backed_identity_pin_proof() {
        Ok(result) => {
            println!(
                "file-backed pinned write completion: id={} len={} sector={}",
                result.write_completion().descriptor_id(),
                result.write_completion().length(),
                result.write_completion().sector()
            );
            println!(
                "file-backed identity distinct: {}",
                result.original_identity() != result.replacement_identity()
            );
            println!(
                "file-backed pinned payload exact: {}",
                result.pinned_sector() == result.payload()
            );
            println!(
                "file-backed replacement untouched: {}",
                result.replacement_sector() != result.payload()
            );
            println!(
                "file-backed checkpoint rejected: {}",
                result.checkpoint_rejected()
            );
            println!("file-backed identity proof: {:?}", result.proof());
            if result.proof() != FILE_BACKED_IDENTITY_PIN_PROOF {
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
