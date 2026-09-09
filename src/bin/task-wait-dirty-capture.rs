use mini_hypervisor::config::VmConfig;
use mini_hypervisor::task::run_bounded_wait_channel_dirty_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_bounded_wait_channel_dirty_guest(VmConfig::default()) {
        Ok(result) => {
            let guest = result.guest();
            println!("wait dirty GSI: {}", guest.gsi());
            println!("wait dirty vector: {:#x}", guest.vector());
            println!("wait dirty LAPIC SPIV: {:#x}", guest.lapic_spiv());
            println!("wait dirty LAPIC LINT0: {:#x}", guest.lapic_lint0());
            println!("wait dirty armed rflags: {:#x}", guest.armed_rflags());
            println!("wait dirty proof: {:?}", guest.proof());
            println!("wait dirty captures: {}", result.captures().len());
            for capture in result.captures() {
                let wait = capture.wait();
                println!(
                    "wait dirty capture {}: bitmap={:?} context_page_dirty={} state={:?} owner={:#x} mismatches={} wakes={} last={:#x}",
                    char::from(capture.stage()),
                    capture.bitmap(),
                    capture.context_page_dirty(),
                    wait.task_a_state(),
                    wait.owner(),
                    wait.mismatch_count(),
                    wait.wake_count(),
                    wait.last_attempt()
                );
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
