use mini_hypervisor::state_snapshot::run_pending_notification_token_replay_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_pending_notification_token_replay_guest() {
        Ok(result) => {
            println!(
                "pending notification transaction: version={} encoded_bytes={} pages={}",
                result.schema_version(),
                result.encoded_len(),
                result.page_count()
            );
            println!("pending notification bars: {:?}", result.bars());
            println!(
                "pending notification token: bar={:#x} queue={} indices={:?}",
                result.token_bar(),
                result.token_queue(),
                result.token_indices()
            );
            println!(
                "pending notification ordinary capture rejected: {}",
                result.ordinary_capture_rejected()
            );
            println!(
                "pending notification capture eventfds: {:?}",
                result.capture_pending()
            );
            println!(
                "pending notification reconstructed eventfds: {:?}",
                result.reconstructed_pending()
            );
            println!(
                "pending notification queues at token: {:?}",
                result.queue_indices_at_token()
            );
            println!(
                "pending notification queues after restore: {:?}",
                result.queue_indices_after_restore()
            );
            println!(
                "pending notification semantic state restored: {}",
                result.restored_notification_pending()
            );
            println!(
                "pending notification backing unchanged at token: {}",
                result.backing_unchanged_at_token()
            );
            println!(
                "pending notification final queues: {:?}",
                result.final_queue_indices()
            );
            println!(
                "pending notification doorbells: {:?}",
                result.doorbell_events()
            );
            println!("pending notification irqfd: {:?}", result.irqfd_signals());
            println!("pending notification proof: {:?}", result.proof());
            println!(
                "pending notification capture rips: {:?}",
                result.capture_rips()
            );
            println!(
                "pending notification boundary rip: {:#x}",
                result.pending_rip()
            );
            println!(
                "pending notification completion rip: {:#x}",
                result.completion_rip()
            );
            println!(
                "pending notification exact restore: {}",
                result.restored().is_exact_match()
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
