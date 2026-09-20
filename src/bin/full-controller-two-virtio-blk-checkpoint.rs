use mini_hypervisor::state_snapshot::{
    run_full_controller_two_virtio_blk_checkpoint_guest, CONTROLLER_CHECKPOINT_PAGE,
    TWO_VIRTIO_BLK_CHECKPOINT_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_full_controller_two_virtio_blk_checkpoint_guest() {
        Ok(result) => {
            let mutation = result.mutation();
            let mutation_controller = mutation.controller();
            let restored = result.restored();
            let restored_controller = restored.controller();

            println!(
                "two-virtio checkpoint page: {:#x}",
                CONTROLLER_CHECKPOINT_PAGE.get()
            );
            println!(
                "two-virtio checkpoint capture: rip={:#x} rflags={:#x}",
                result.capture().rip(),
                result.capture().rflags()
            );
            println!("two-virtio checkpoint bars: {:?}", result.captured_bars());
            println!(
                "two-virtio checkpoint captured statuses: {:?}",
                result.captured_statuses()
            );
            println!(
                "two-virtio checkpoint mutation exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} first={} second={}",
                mutation_controller
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                mutation_controller.vcpu_exact(),
                mutation_controller.master_pic_exact(),
                mutation_controller.slave_pic_exact(),
                mutation_controller.ioapic_exact(),
                mutation_controller.lapic_exact(),
                mutation.device_exact(result.captured_bars()[0]).unwrap_or(false),
                mutation.device_exact(result.captured_bars()[1]).unwrap_or(false),
            );
            println!(
                "two-virtio checkpoint restore exact: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={} first={} second={}",
                restored_controller
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                restored_controller.vcpu_exact(),
                restored_controller.master_pic_exact(),
                restored_controller.slave_pic_exact(),
                restored_controller.ioapic_exact(),
                restored_controller.lapic_exact(),
                restored.device_exact(result.captured_bars()[0]).unwrap_or(false),
                restored.device_exact(result.captured_bars()[1]).unwrap_or(false),
            );
            println!(
                "two-virtio checkpoint restored statuses: {:?}",
                result.restored_statuses()
            );
            println!("two-virtio checkpoint proof: {:?}", result.proof());
            println!(
                "two-virtio checkpoint completion rflags: {:#x}",
                result.completion_rflags()
            );

            if result.proof() == TWO_VIRTIO_BLK_CHECKPOINT_PROOF {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
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
