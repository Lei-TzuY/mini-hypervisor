use mini_hypervisor::state_snapshot::run_transaction_coupled_dual_device_replay_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_transaction_coupled_dual_device_replay_guest() {
        Ok(result) => {
            let mutation = result.mutation();
            let restored = result.restored();
            println!(
                "coupled transaction: version={} encoded_bytes={} checkpoint_bytes={} registration_pair_bytes={}",
                result.transaction_version(),
                result.encoded_len(),
                result.checkpoint_encoded_len(),
                result.registration_pair_encoded_len()
            );
            println!("coupled bars: {:?}", result.bars());
            println!(
                "coupled mutation: exact={} first-device={:?} second-device={:?}",
                mutation.is_exact_match(),
                mutation.device_exact(result.bars()[0]),
                mutation.device_exact(result.bars()[1])
            );
            println!("coupled restore exact: {}", restored.is_exact_match());
            println!(
                "coupled replay queues: {:?}",
                result.replay_queue_indices()
            );
            println!("coupled replay doorbells: {:?}", result.doorbell_events());
            println!("coupled replay irqfd: {:?}", result.irqfd_signals());
            println!(
                "coupled replay data exact: [{}, {}]",
                result.readback()[0] == result.backing()[0],
                result.readback()[1] == result.backing()[1]
            );
            println!("coupled replay proof: {:?}", result.proof());
            println!(
                "coupled replay completion rflags: {:#x}",
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
