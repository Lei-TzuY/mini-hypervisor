use mini_hypervisor::state_snapshot::run_acceleration_checkpoint_quiescence_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_acceleration_checkpoint_quiescence_guest() {
        Ok(result) => {
            println!(
                "acceleration checkpoint rejected pending: {:?}",
                result.rejected_pending()
            );
            println!(
                "acceleration checkpoint pending preserved: {:?}",
                result.pending_after_rejection()
            );
            println!(
                "acceleration checkpoint preserved doorbell count: {}",
                result.preserved_doorbell_count()
            );
            println!(
                "acceleration checkpoint serviced queues: {:?}",
                result.serviced_queue_indices()
            );
            println!(
                "acceleration checkpoint captured queues: {:?}",
                result.captured_queue_indices()
            );
            println!("acceleration checkpoint proof: {:?}", result.proof());
            println!(
                "acceleration checkpoint completion rflags: {:#x}",
                result.completion_rflags()
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
