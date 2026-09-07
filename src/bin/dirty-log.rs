use mini_hypervisor::config::VmConfig;
use mini_hypervisor::kvm::KvmBackend;
use std::process::ExitCode;

fn main() -> ExitCode {
    match KvmBackend::run_dirty_log_guest(VmConfig::default()) {
        Ok((first, second, values, proof, report)) => {
            println!("dirty-log first bitmap: {first:?}");
            println!("dirty-log second bitmap: {second:?}");
            println!("dirty-log guest bytes: {values:?}");
            println!("dirty-log proof: {proof:?}");
            println!("{report}");
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
