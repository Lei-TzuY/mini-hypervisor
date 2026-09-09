use mini_hypervisor::config::VmConfig;
use mini_hypervisor::state_snapshot::{
    run_multi_page_checkpoint_guest, MULTI_PAGE_CHECKPOINT_CONTROL_PAGE,
    MULTI_PAGE_CHECKPOINT_DATA_PAGE, MULTI_PAGE_CHECKPOINT_STACK_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_multi_page_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            println!(
                "multi-page checkpoint capture: {}",
                result.checkpoint_report()
            );
            println!(
                "multi-page checkpoint pages: {:#x},{:#x},{:#x}",
                MULTI_PAGE_CHECKPOINT_CONTROL_PAGE.get(),
                MULTI_PAGE_CHECKPOINT_DATA_PAGE.get(),
                MULTI_PAGE_CHECKPOINT_STACK_PAGE.get()
            );
            println!(
                "multi-page checkpoint corruption: control={} data={} stack={} vcpu={}",
                result
                    .corruption()
                    .page_exact(MULTI_PAGE_CHECKPOINT_CONTROL_PAGE)
                    .unwrap_or(false),
                result
                    .corruption()
                    .page_exact(MULTI_PAGE_CHECKPOINT_DATA_PAGE)
                    .unwrap_or(false),
                result
                    .corruption()
                    .page_exact(MULTI_PAGE_CHECKPOINT_STACK_PAGE)
                    .unwrap_or(false),
                result.corruption().vcpu().is_exact_match()
            );
            println!(
                "multi-page checkpoint restore: control={} data={} stack={} vcpu={}",
                result
                    .restored()
                    .page_exact(MULTI_PAGE_CHECKPOINT_CONTROL_PAGE)
                    .unwrap_or(false),
                result
                    .restored()
                    .page_exact(MULTI_PAGE_CHECKPOINT_DATA_PAGE)
                    .unwrap_or(false),
                result
                    .restored()
                    .page_exact(MULTI_PAGE_CHECKPOINT_STACK_PAGE)
                    .unwrap_or(false),
                result.restored().vcpu().is_exact_match()
            );
            println!("multi-page checkpoint proof: {:?}", result.proof());
            println!(
                "multi-page checkpoint terminal: {}",
                result.terminal_report()
            );
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
