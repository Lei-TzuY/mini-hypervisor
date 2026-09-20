use mini_hypervisor::state_snapshot::{
    run_versioned_full_controller_two_virtio_blk_checkpoint_guest, CONTROLLER_CHECKPOINT_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_versioned_full_controller_two_virtio_blk_checkpoint_guest() {
        Ok(result) => {
            let checkpoint = result.checkpoint();
            let mutation = checkpoint.mutation();
            let mutation_controller = mutation.controller();
            let restored = checkpoint.restored();
            let restored_controller = restored.controller();

            println!(
                "versioned two-virtio schema: version={} encoded_bytes={} pages={} msrs={} bars={:?} backing_each={} canonical={}",
                result.schema_version(),
                result.encoded_len(),
                result.page_count(),
                result.msr_count(),
                result.bars(),
                result.backing_len_each(),
                result.canonical_roundtrip()
            );
            println!(
                "versioned two-virtio checkpoint page: {:#x}",
                CONTROLLER_CHECKPOINT_PAGE.get()
            );
            println!(
                "versioned two-virtio captured statuses: {:?}",
                checkpoint.captured_statuses()
            );
            println!(
                "versioned two-virtio mutation exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} first={} second={}",
                mutation_controller
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                mutation_controller.vcpu_exact(),
                mutation_controller.master_pic_exact(),
                mutation_controller.slave_pic_exact(),
                mutation_controller.ioapic_exact(),
                mutation_controller.lapic_exact(),
                mutation
                    .device_exact(checkpoint.captured_bars()[0])
                    .unwrap_or(false),
                mutation
                    .device_exact(checkpoint.captured_bars()[1])
                    .unwrap_or(false),
            );
            println!(
                "versioned two-virtio restore exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} first={} second={}",
                restored_controller
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                restored_controller.vcpu_exact(),
                restored_controller.master_pic_exact(),
                restored_controller.slave_pic_exact(),
                restored_controller.ioapic_exact(),
                restored_controller.lapic_exact(),
                restored
                    .device_exact(checkpoint.captured_bars()[0])
                    .unwrap_or(false),
                restored
                    .device_exact(checkpoint.captured_bars()[1])
                    .unwrap_or(false),
            );
            println!(
                "versioned two-virtio restored statuses: {:?}",
                checkpoint.restored_statuses()
            );
            println!("versioned two-virtio proof: {:?}", checkpoint.proof());
            println!(
                "versioned two-virtio completion rflags: {:#x}",
                checkpoint.completion_rflags()
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
