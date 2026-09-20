use mini_hypervisor::state_snapshot::run_pending_completion_token_replay_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_pending_completion_token_replay_guest() {
        Ok(result) => {
            println!(
                "pending completion transaction: version={} encoded_bytes={} pages={}",
                result.schema_version(),
                result.encoded_len(),
                result.page_count()
            );
            println!("pending completion bars: {:?}", result.bars());
            println!(
                "pending completion token: bar={:#x} queue={} indices={:?}",
                result.token_bar(),
                result.token_queue(),
                result.token_indices()
            );
            println!(
                "pending completion ordinary capture rejected: {}",
                result.ordinary_capture_rejected()
            );
            println!(
                "pending completion acceleration: capture={:?} reconstructed={:?}",
                result.capture_pending(),
                result.reconstructed_pending()
            );
            println!(
                "pending completion queues: token={:?} restored={:?} final={:?}",
                result.queue_indices_at_token(),
                result.queue_indices_after_restore(),
                result.final_queue_indices()
            );
            println!(
                "pending completion counts: doorbells={:?} irqfd={:?}",
                result.doorbell_events(),
                result.irqfd_signals()
            );
            println!(
                "pending completion restore exact: {}",
                result.restored().is_exact_match()
            );
            println!("pending completion proof: {:?}", result.proof());
            println!(
                "pending completion rips: capture={:?} pending={:#x} completion={:#x}",
                result.capture_rips(),
                result.pending_rip(),
                result.completion_rip()
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
