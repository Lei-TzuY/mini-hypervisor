use mini_hypervisor::state_snapshot::{
    run_restored_multi_producer_data_plane_replay_guest,
    RESTORED_MULTI_PRODUCER_FIRST_PROOF, RESTORED_MULTI_PRODUCER_SECOND_PROOF,
    TRANSACTION_COUPLED_FIRST_PAGE, TRANSACTION_COUPLED_SECOND_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_restored_multi_producer_data_plane_replay_guest() {
        Ok(result) => {
            println!(
                "multi-producer replay transaction: version={} encoded_bytes={} pages={} canonical={}",
                result.schema_version(),
                result.encoded_len(),
                result.page_count(),
                result.canonical_roundtrip()
            );
            println!("multi-producer replay bars: {:?}", result.bars());
            println!(
                "multi-producer replay ioapic: [{:#x}, {:#x}]",
                result.ioapic_entries()[0],
                result.ioapic_entries()[1]
            );
            println!(
                "multi-producer replay capture pending: {:?}",
                result.capture_pending()
            );
            println!(
                "multi-producer replay reconstructed pending: {:?}",
                result.reconstructed_pending()
            );
            println!(
                "multi-producer replay mutation: exact={} queue-pages=[{:?}, {:?}]",
                result.mutation().is_exact_match(),
                result
                    .mutation()
                    .controller()
                    .page_exact(TRANSACTION_COUPLED_FIRST_PAGE),
                result
                    .mutation()
                    .controller()
                    .page_exact(TRANSACTION_COUPLED_SECOND_PAGE)
            );
            println!(
                "multi-producer replay restore exact: {}",
                result.restored().is_exact_match()
            );
            println!("multi-producer replay queues: {:?}", result.queue_indices());
            println!(
                "multi-producer replay doorbells: {:?}",
                result.doorbell_events()
            );
            println!("multi-producer replay irqfd: {:?}", result.irqfd_signals());
            println!(
                "multi-producer replay payload exact: [{}, {}]",
                result.readback()[0] == result.write_payloads()[0]
                    && result.backing()[0] == result.write_payloads()[0],
                result.readback()[1] == result.write_payloads()[1]
                    && result.backing()[1] == result.write_payloads()[1]
            );
            println!(
                "multi-producer replay backing distinct: {}",
                result.backing()[0] != result.backing()[1]
            );
            println!(
                "multi-producer replay proofs: {:?} / {:?}",
                result.producer_proofs()[0],
                result.producer_proofs()[1]
            );
            println!(
                "multi-producer replay capture rips: [{:#x}, {:#x}]",
                result.capture_rips()[0],
                result.capture_rips()[1]
            );
            println!(
                "multi-producer replay completion rips: [{:#x}, {:#x}]",
                result.completion_rips()[0],
                result.completion_rips()[1]
            );
            assert_eq!(result.producer_proofs()[0], RESTORED_MULTI_PRODUCER_FIRST_PROOF);
            assert_eq!(
                result.producer_proofs()[1],
                RESTORED_MULTI_PRODUCER_SECOND_PROOF
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
