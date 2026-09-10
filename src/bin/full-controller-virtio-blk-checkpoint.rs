use mini_hypervisor::state_snapshot::{
    run_full_controller_virtio_blk_checkpoint_guest, FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_full_controller_virtio_blk_checkpoint_guest() {
        Ok(result) => {
            let mutation = result.mutation();
            let mutation_controller = mutation.controller();
            let restored = result.restored();
            let restored_controller = restored.controller();

            println!(
                "full-controller virtio-blk checkpoint page: {:#x}",
                FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE.get()
            );
            println!(
                "full-controller virtio-blk capture: rip={:#x} rflags={:#x}",
                result.capture_rip(),
                result.capture_rflags()
            );
            println!(
                "full-controller virtio-blk captured queue: avail={} used={}",
                result.captured_avail_idx(),
                result.captured_used_idx()
            );
            println!(
                "full-controller virtio-blk mutation exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} device={}",
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
                "full-controller virtio-blk mutation proof: {:?}",
                result.mutation_proof()
            );
            println!(
                "full-controller virtio-blk mutation IRQ lifecycle: assert={} deassert={}",
                result.mutation_assert_count(),
                result.mutation_deassert_count()
            );
            println!(
                "full-controller virtio-blk restore exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} device={}",
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
                "full-controller virtio-blk replay queue: avail={} used={}",
                result.replay_avail_idx(),
                result.replay_used_idx()
            );
            println!(
                "full-controller virtio-blk replay proof: {:?}",
                result.replay_proof()
            );
            println!(
                "full-controller virtio-blk replay IRQ lifecycle: assert={} deassert={}",
                result.replay_assert_count(),
                result.replay_deassert_count()
            );
            println!(
                "full-controller virtio-blk replay rflags: {:#x}",
                result.replay_rflags()
            );
            println!(
                "full-controller virtio-blk backing/readback match: {}",
                result.backing() == result.readback()
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
