use mini_hypervisor::config::VmConfig;
use mini_hypervisor::state_snapshot::{
    run_virtio_blk_checkpoint_guest, VIRTIO_BLK_CHECKPOINT_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_virtio_blk_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            println!(
                "virtio-blk checkpoint page: {:#x}",
                VIRTIO_BLK_CHECKPOINT_PAGE.get()
            );
            println!("virtio-blk checkpoint capture: {}", result.capture_report());
            println!(
                "virtio-blk checkpoint captured queue: avail={} used={}",
                result.captured_avail_idx(),
                result.captured_used_idx()
            );
            println!(
                "virtio-blk checkpoint capture proof: {:?}",
                result.capture_proof()
            );
            println!(
                "virtio-blk checkpoint mutation: {}",
                result.mutation_report()
            );
            println!(
                "virtio-blk checkpoint mutation exact: page={} vcpu={} device={}",
                result
                    .mutation()
                    .page_exact(VIRTIO_BLK_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                result.mutation().vcpu_exact(),
                result.mutation().device_exact()
            );
            println!(
                "virtio-blk checkpoint mutation proof: {:?}",
                result.mutation_proof()
            );
            println!(
                "virtio-blk checkpoint restore exact: page={} vcpu={} device={}",
                result
                    .restored()
                    .page_exact(VIRTIO_BLK_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                result.restored().vcpu_exact(),
                result.restored().device_exact()
            );
            println!(
                "virtio-blk checkpoint replay: {}",
                result.replay_report()
            );
            println!(
                "virtio-blk checkpoint replay queue: avail={} used={}",
                result.replay_avail_idx(),
                result.replay_used_idx()
            );
            println!(
                "virtio-blk checkpoint replay proof: {:?}",
                result.replay_proof()
            );
            println!(
                "virtio-blk checkpoint backing/readback match: {}",
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
