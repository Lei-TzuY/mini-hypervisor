use mini_hypervisor::state_snapshot::{
    run_versioned_full_controller_virtio_blk_checkpoint_guest,
    FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_versioned_full_controller_virtio_blk_checkpoint_guest() {
        Ok(result) => {
            let checkpoint = result.checkpoint();
            let mutation = checkpoint.mutation();
            let mutation_controller = mutation.controller();
            let restored = checkpoint.restored();
            let restored_controller = restored.controller();

            println!(
                "versioned full-controller virtio-blk schema: version={} encoded_bytes={} pages={} msrs={} bar={:#x} backing_bytes={} canonical={}",
                result.schema_version(),
                result.encoded_len(),
                result.page_count(),
                result.msr_count(),
                result.bar0(),
                result.backing_len(),
                result.canonical_roundtrip()
            );
            println!(
                "versioned full-controller virtio-blk checkpoint page: {:#x}",
                FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE.get()
            );
            println!(
                "versioned full-controller virtio-blk capture: rip={:#x} rflags={:#x}",
                checkpoint.capture_rip(),
                checkpoint.capture_rflags()
            );
            println!(
                "versioned full-controller virtio-blk captured queue: avail={} used={}",
                checkpoint.captured_avail_idx(),
                checkpoint.captured_used_idx()
            );
            println!(
                "versioned full-controller virtio-blk mutation exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} device={}",
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
                "versioned full-controller virtio-blk mutation proof: {:?}",
                checkpoint.mutation_proof()
            );
            println!(
                "versioned full-controller virtio-blk mutation IRQ lifecycle: assert={} deassert={}",
                checkpoint.mutation_assert_count(),
                checkpoint.mutation_deassert_count()
            );
            println!(
                "versioned full-controller virtio-blk restore exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} device={}",
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
                "versioned full-controller virtio-blk replay queue: avail={} used={}",
                checkpoint.replay_avail_idx(),
                checkpoint.replay_used_idx()
            );
            println!(
                "versioned full-controller virtio-blk replay proof: {:?}",
                checkpoint.replay_proof()
            );
            println!(
                "versioned full-controller virtio-blk replay IRQ lifecycle: assert={} deassert={}",
                checkpoint.replay_assert_count(),
                checkpoint.replay_deassert_count()
            );
            println!(
                "versioned full-controller virtio-blk replay rflags: {:#x}",
                checkpoint.replay_rflags()
            );
            println!(
                "versioned full-controller virtio-blk backing/readback match: {}",
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
