use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::state_snapshot::{
    run_versioned_two_device_checkpoint_transaction_guest, CONTROLLER_CHECKPOINT_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_versioned_two_device_checkpoint_transaction_guest() {
        Ok(result) => {
            println!(
                "two-device transaction schema: version={} encoded_bytes={} checkpoint_version={} checkpoint_bytes={} registration_pair_version={} registration_pair_bytes={} registration_versions={:?} pages={} msrs={} bars={:?} backing_each={} canonical={}",
                result.transaction_version(),
                result.encoded_len(),
                result.checkpoint_schema_version(),
                result.checkpoint_encoded_len(),
                result.registration_pair_schema_version(),
                result.registration_pair_encoded_len(),
                result.registration_versions(),
                result.page_count(),
                result.msr_count(),
                result.bars(),
                result.backing_len_each(),
                result.canonical_roundtrip()
            );

            let checkpoint = result.checkpoint();
            let mutation = checkpoint.mutation();
            let mutation_controller = mutation.controller();
            let restored = checkpoint.restored();
            let restored_controller = restored.controller();

            println!(
                "two-device transaction checkpoint page: {:#x}",
                CONTROLLER_CHECKPOINT_PAGE.get()
            );
            println!(
                "two-device transaction captured statuses: {:?}",
                checkpoint.captured_statuses()
            );
            println!(
                "two-device transaction mutation exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} first={} second={}",
                mutation_controller
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                mutation_controller.vcpu_exact(),
                mutation_controller.master_pic_exact(),
                mutation_controller.slave_pic_exact(),
                mutation_controller.ioapic_exact(),
                mutation_controller.lapic_exact(),
                mutation.device_exact(result.bars()[0]).unwrap_or(false),
                mutation.device_exact(result.bars()[1]).unwrap_or(false),
            );
            println!(
                "two-device transaction restore exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} first={} second={}",
                restored_controller
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                restored_controller.vcpu_exact(),
                restored_controller.master_pic_exact(),
                restored_controller.slave_pic_exact(),
                restored_controller.ioapic_exact(),
                restored_controller.lapic_exact(),
                restored.device_exact(result.bars()[0]).unwrap_or(false),
                restored.device_exact(result.bars()[1]).unwrap_or(false),
            );
            println!(
                "two-device transaction restored statuses: {:?}",
                checkpoint.restored_statuses()
            );
            println!(
                "two-device transaction checkpoint proof: {:?}",
                checkpoint.proof()
            );
            println!(
                "two-device transaction checkpoint rflags: {:#x}",
                checkpoint.completion_rflags()
            );
            println!(
                "two-device transaction acceleration: doorbells={:?} gsis={:?} vectors={:?} events={:?}",
                result.acceleration_doorbells(),
                result.acceleration_gsis(),
                result.acceleration_vectors(),
                result.acceleration_generation_events()
            );
            println!(
                "two-device transaction acceleration proof: {:?}",
                result.acceleration_proof()
            );
            println!(
                "two-device transaction acceleration rflags: {:#x}",
                result.acceleration_completion_rflags()
            );

            if checkpoint.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE == 0
                || result.acceleration_completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE == 0
            {
                eprintln!("error: transaction proof completed with interrupts disabled");
                return ExitCode::FAILURE;
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
