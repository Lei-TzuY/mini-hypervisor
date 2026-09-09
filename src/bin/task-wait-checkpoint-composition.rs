use mini_hypervisor::config::VmConfig;
use mini_hypervisor::task::run_bounded_wait_channel_checkpoint_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_bounded_wait_channel_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            let captured = result.captured();
            let wait = captured.wait();
            let queue = captured.queue();
            println!("wait checkpoint capture: {}", result.checkpoint_report());
            println!(
                "wait checkpoint ownership: state={:?} owner={:#x} mismatches={} wakes={} last={:#x} selected={:?}",
                wait.task_a_state(),
                wait.owner(),
                wait.mismatch_count(),
                wait.wake_count(),
                wait.last_attempt(),
                queue.selected()
            );
            println!(
                "wait checkpoint corruption: page_exact={} vcpu_exact={} wait_exact={} queue_exact={} task_a_exact={} task_b_exact={}",
                result.corruption().machine().page_exact(),
                result.corruption().machine().vcpu().is_exact_match(),
                result.corruption().wait_exact(),
                result.corruption().queue_exact(),
                result.corruption().task_a_exact(),
                result.corruption().task_b_exact()
            );
            println!(
                "wait checkpoint restore: page_exact={} vcpu_exact={} wait_exact={} queue_exact={} task_a_exact={} task_b_exact={}",
                result.restored().machine().page_exact(),
                result.restored().machine().vcpu().is_exact_match(),
                result.restored().wait_exact(),
                result.restored().queue_exact(),
                result.restored().task_a_exact(),
                result.restored().task_b_exact()
            );
            println!("wait checkpoint proof: {:?}", result.guest().proof());
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
