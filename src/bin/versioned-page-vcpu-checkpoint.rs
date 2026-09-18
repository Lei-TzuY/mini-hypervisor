use mini_hypervisor::config::VmConfig;
use mini_hypervisor::state_snapshot::{
    run_versioned_page_vcpu_checkpoint_guest, MULTI_PAGE_CHECKPOINT_CONTROL_PAGE,
    MULTI_PAGE_CHECKPOINT_DATA_PAGE, MULTI_PAGE_CHECKPOINT_STACK_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_versioned_page_vcpu_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            let checkpoint = result.checkpoint();
            println!("versioned checkpoint schema: version={} encoded_bytes={} msrs={}", result.schema_version(), result.encoded_len(), result.msr_count());
            println!("versioned checkpoint pages: {:#x},{:#x},{:#x}", MULTI_PAGE_CHECKPOINT_CONTROL_PAGE.get(), MULTI_PAGE_CHECKPOINT_DATA_PAGE.get(), MULTI_PAGE_CHECKPOINT_STACK_PAGE.get());
            println!("versioned checkpoint corruption: control={} data={} stack={} vcpu={}", checkpoint.corruption().page_exact(MULTI_PAGE_CHECKPOINT_CONTROL_PAGE).unwrap_or(false), checkpoint.corruption().page_exact(MULTI_PAGE_CHECKPOINT_DATA_PAGE).unwrap_or(false), checkpoint.corruption().page_exact(MULTI_PAGE_CHECKPOINT_STACK_PAGE).unwrap_or(false), checkpoint.corruption().vcpu().is_exact_match());
            println!("versioned checkpoint restore: control={} data={} stack={} vcpu={}", checkpoint.restored().page_exact(MULTI_PAGE_CHECKPOINT_CONTROL_PAGE).unwrap_or(false), checkpoint.restored().page_exact(MULTI_PAGE_CHECKPOINT_DATA_PAGE).unwrap_or(false), checkpoint.restored().page_exact(MULTI_PAGE_CHECKPOINT_STACK_PAGE).unwrap_or(false), checkpoint.restored().vcpu().is_exact_match());
            println!("versioned checkpoint proof: {:?}", checkpoint.proof());
            println!("versioned checkpoint capture: {}", checkpoint.checkpoint_report());
            println!("versioned checkpoint terminal: {}", checkpoint.terminal_report());
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
