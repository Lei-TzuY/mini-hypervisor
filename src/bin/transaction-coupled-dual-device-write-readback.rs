use mini_hypervisor::state_snapshot::run_transaction_coupled_dual_device_write_readback_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_transaction_coupled_dual_device_write_readback_guest() {
        Ok(result) => {
            println!(
                "coupled write/readback transaction: version={} encoded_bytes={}",
                result.transaction_version(),
                result.encoded_len()
            );
            println!("coupled write/readback bars: {:?}", result.bars());
            println!(
                "coupled write/readback mutation: exact={} first-device={:?} second-device={:?}",
                result.mutation().is_exact_match(),
                result.mutation().device_exact(result.bars()[0]),
                result.mutation().device_exact(result.bars()[1])
            );
            println!(
                "coupled write/readback restore exact: {}",
                result.restored().is_exact_match()
            );
            println!(
                "coupled write/readback queues: {:?}",
                result.queue_indices()
            );
            println!(
                "coupled write/readback doorbells: {:?}",
                result.doorbell_events()
            );
            println!("coupled write/readback irqfd: {:?}", result.irqfd_signals());
            println!(
                "coupled write/readback payload exact: [{}, {}]",
                result.readback()[0] == result.write_payloads()[0]
                    && result.backing()[0] == result.write_payloads()[0],
                result.readback()[1] == result.write_payloads()[1]
                    && result.backing()[1] == result.write_payloads()[1]
            );
            println!(
                "coupled write/readback backing distinct: {}",
                result.backing()[0] != result.backing()[1]
            );
            println!("coupled write/readback proof: {:?}", result.proof());
            println!(
                "coupled write/readback completion rflags: {:#x}",
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
