use mini_hypervisor::state_snapshot::{
    run_versioned_checkpoint_transaction_guest, FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_versioned_checkpoint_transaction_guest() {
        Ok(result) => {
            let checkpoint = result.checkpoint();
            let mutation = checkpoint.mutation();
            let mutation_controller = mutation.controller();
            let restored = checkpoint.restored();
            let restored_controller = restored.controller();

            println!(
                "checkpoint transaction schema: version={} encoded_bytes={} checkpoint_bytes={} registration_bytes={} canonical={}",
                result.transaction_version(),
                result.encoded_len(),
                result.checkpoint_encoded_len(),
                result.registration_encoded_len(),
                result.canonical_roundtrip()
            );
            println!(
                "checkpoint transaction nested: checkpoint_version={} registration_version={} pages={} msrs={} bar={:#x} backing_bytes={}",
                result.checkpoint_schema_version(),
                result.registration_schema_version(),
                result.page_count(),
                result.msr_count(),
                result.bar0(),
                result.backing_len()
            );
            println!(
                "checkpoint transaction page: {:#x}",
                FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE.get()
            );
            println!(
                "checkpoint transaction descriptor: doorbell={:#x} len={} datamatch={:#x} gsi={}",
                checkpoint.doorbell_gpa(),
                checkpoint.doorbell_length(),
                checkpoint.doorbell_datamatch(),
                checkpoint.gsi()
            );
            println!(
                "checkpoint transaction capture: rip={:#x} rflags={:#x} queue={}/{}",
                checkpoint.capture_rip(),
                checkpoint.capture_rflags(),
                checkpoint.captured_avail_idx(),
                checkpoint.captured_used_idx()
            );
            println!(
                "checkpoint transaction mutation exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} device={}",
                mutation_controller
                    .page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                mutation_controller.vcpu_exact(),
                mutation_controller.master_pic_exact(),
                mutation_controller.slave_pic_exact(),
                mutation_controller.ioapic_exact(),
                mutation_controller.lapic_exact(),
                mutation.device_exact()
            );
            println!(
                "checkpoint transaction mutation transport: doorbell-events={} irqfd-signals={}",
                checkpoint.mutation_doorbell_events(),
                checkpoint.mutation_irqfd_signals()
            );
            println!(
                "checkpoint transaction mutation proof: {:?}",
                checkpoint.mutation_proof()
            );
            println!(
                "checkpoint transaction restore exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} device={}",
                restored_controller
                    .page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                restored_controller.vcpu_exact(),
                restored_controller.master_pic_exact(),
                restored_controller.slave_pic_exact(),
                restored_controller.ioapic_exact(),
                restored_controller.lapic_exact(),
                restored.device_exact()
            );
            println!(
                "checkpoint transaction replay transport: doorbell-events={} irqfd-signals={}",
                checkpoint.replay_doorbell_events(),
                checkpoint.replay_irqfd_signals()
            );
            println!(
                "checkpoint transaction replay queue: avail={} used={}",
                checkpoint.replay_avail_idx(),
                checkpoint.replay_used_idx()
            );
            println!(
                "checkpoint transaction replay proof: {:?}",
                checkpoint.replay_proof()
            );
            println!(
                "checkpoint transaction replay rflags: {:#x}",
                checkpoint.replay_rflags()
            );
            println!(
                "checkpoint transaction backing/readback match: {}",
                checkpoint.backing() == checkpoint.readback()
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
