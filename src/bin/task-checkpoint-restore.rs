use mini_hypervisor::config::VmConfig;
use mini_hypervisor::state_snapshot::run_bounded_checkpoint_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_bounded_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            println!("checkpoint capture: {}", result.checkpoint_report());
            println!(
                "checkpoint corruption: page_exact={} vcpu_exact={}",
                result.corruption().page_exact(),
                result.corruption().vcpu().is_exact_match()
            );
            println!(
                "checkpoint restore: page_exact={} vcpu_exact={}",
                result.restored().page_exact(),
                result.restored().vcpu().is_exact_match()
            );
            println!("checkpoint marker: {:#x}", result.restored_marker());
            println!("checkpoint proof: {:?}", result.proof());
            println!("checkpoint terminal: {}", result.terminal_report());
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
