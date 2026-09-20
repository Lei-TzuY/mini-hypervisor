use mini_hypervisor::state_snapshot::{
    run_versioned_two_vcpu_two_device_transaction_guest, TWO_VCPU_CHECKPOINT_FIRST_ID,
    TWO_VCPU_CHECKPOINT_SECOND_ID,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_versioned_two_vcpu_two_device_transaction_guest() {
        Ok(result) => {
            let transaction = result.transaction();
            println!(
                "versioned two-vCPU two-device schema: transaction={} checkpoint={} controller={} registration-pair={} registrations={:?}",
                result.schema_version(),
                result.checkpoint_version(),
                result.controller_version(),
                result.registration_pair_version(),
                result.registration_versions()
            );
            println!(
                "versioned two-vCPU two-device bytes: total={} checkpoint={} registration-pair={} canonical={}",
                result.encoded_len(),
                result.checkpoint_encoded_len(),
                result.registration_pair_encoded_len(),
                result.canonical_roundtrip()
            );
            println!(
                "versioned two-vCPU two-device ownership: vcpus=[{},{}] mp={:?} pages={} msrs={:?} bars={:#x},{:#x} backing-each={}",
                result.vcpu_ids()[0].get(),
                result.vcpu_ids()[1].get(),
                result.mp_states(),
                result.page_count(),
                result.msr_counts(),
                result.bars()[0],
                result.bars()[1],
                result.backing_len_each()
            );
            println!(
                "versioned two-vCPU two-device statuses: captured={:?} restored={:?}",
                transaction.captured_statuses(),
                transaction.restored_statuses()
            );
            println!(
                "versioned two-vCPU two-device quiescence: capture={:?} reconstructed={:?}",
                transaction.capture_pending(),
                transaction.reconstructed_pending()
            );
            println!(
                "versioned two-vCPU two-device exact: mutation={} restored={} vcpu0={} vcpu1={}",
                transaction.mutation().is_exact_match(),
                transaction.restored().is_exact_match(),
                transaction
                    .restored()
                    .controller()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID)
                    == Some(true),
                transaction
                    .restored()
                    .controller()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
                    == Some(true)
            );
            println!(
                "versioned two-vCPU two-device rips: capture={:#x},{:#x} completion={:#x},{:#x}",
                transaction.first_capture_rip(),
                transaction.second_capture_rip(),
                transaction.first_completion_rip(),
                transaction.second_completion_rip()
            );
            println!(
                "versioned two-vCPU two-device proofs: {:?},{:?}",
                transaction.first_proof(),
                transaction.second_proof()
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
