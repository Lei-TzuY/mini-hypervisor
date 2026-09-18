use mini_hypervisor::state_snapshot::{
    run_full_controller_virtio_blk_host_registration_reconstruction_guest,
    FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_full_controller_virtio_blk_host_registration_reconstruction_guest() {
        Ok(result) => {
            let mutation = result.mutation();
            let mc = mutation.controller();
            let restored = result.restored();
            let rc = restored.controller();
            println!("host-registration checkpoint page: {:#x}", FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE.get());
            println!("host-registration descriptor: doorbell={:#x} len={} datamatch={:#x} gsi={}",
                result.doorbell_gpa(), result.doorbell_length(), result.doorbell_datamatch(), result.gsi());
            println!("host-registration capture: rip={:#x} rflags={:#x}", result.capture_rip(), result.capture_rflags());
            println!("host-registration captured queue: avail={} used={}", result.captured_avail_idx(), result.captured_used_idx());
            println!("host-registration mutation exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} device={}",
                mc.page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE).unwrap_or(false),
                mc.vcpu_exact(), mc.master_pic_exact(), mc.slave_pic_exact(), mc.ioapic_exact(), mc.lapic_exact(), mutation.device_exact());
            println!("host-registration mutation transport: doorbell-events={} irqfd-signals={}", result.mutation_doorbell_events(), result.mutation_irqfd_signals());
            println!("host-registration mutation proof: {:?}", result.mutation_proof());
            println!("host-registration restore exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} device={}",
                rc.page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE).unwrap_or(false),
                rc.vcpu_exact(), rc.master_pic_exact(), rc.slave_pic_exact(), rc.ioapic_exact(), rc.lapic_exact(), restored.device_exact());
            println!("host-registration replay transport: doorbell-events={} irqfd-signals={}", result.replay_doorbell_events(), result.replay_irqfd_signals());
            println!("host-registration replay queue: avail={} used={}", result.replay_avail_idx(), result.replay_used_idx());
            println!("host-registration replay proof: {:?}", result.replay_proof());
            println!("host-registration replay rflags: {:#x}", result.replay_rflags());
            println!("host-registration backing/readback match: {}", result.backing() == result.readback());
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
