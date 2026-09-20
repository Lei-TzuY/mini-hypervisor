use mini_hypervisor::state_snapshot::{
    run_two_vcpu_two_device_transaction_guest, TWO_VCPU_CHECKPOINT_FIRST_ID,
    TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE, TWO_VCPU_CHECKPOINT_OWNERSHIP_SET,
    TWO_VCPU_CHECKPOINT_SECOND_ID, TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
    TWO_VCPU_CHECKPOINT_SHARED_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_two_device_transaction_guest() {
        Ok(result) => {
            let mutation = result.mutation();
            let restored = result.restored();
            let mutation_controller = mutation.controller();
            let restored_controller = restored.controller();
            let bars = result.bars();

            println!("two-vCPU two-device bars: {:#x},{:#x}", bars[0], bars[1]);
            println!(
                "two-vCPU two-device statuses: captured={:?} restored={:?}",
                result.captured_statuses(),
                result.restored_statuses()
            );
            println!(
                "two-vCPU two-device quiescence: capture={:?} reconstructed={:?}",
                result.capture_pending(),
                result.reconstructed_pending()
            );
            println!(
                "two-vCPU two-device mutation: shared={:?} first-stack={:?} second-stack={:?} vcpu0={:?} vcpu1={:?} mp0={:?} mp1={:?} master={} slave={} ioapic={} lapic0={:?} lapic1={:?} dev0={:?} dev1={:?}",
                mutation_controller.page_exact(TWO_VCPU_CHECKPOINT_SHARED_PAGE),
                mutation_controller.page_exact(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE),
                mutation_controller.page_exact(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE),
                mutation_controller.vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                mutation_controller.vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                mutation_controller.mp_state_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                mutation_controller.mp_state_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                mutation_controller.master_pic_exact(),
                mutation_controller.slave_pic_exact(),
                mutation_controller.ioapic_exact(),
                mutation_controller.lapic_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                mutation_controller.lapic_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                mutation.device_exact(bars[0]),
                mutation.device_exact(bars[1])
            );
            println!(
                "two-vCPU two-device restore: shared={:?} first-stack={:?} second-stack={:?} vcpu0={:?} vcpu1={:?} mp0={:?} mp1={:?} master={} slave={} ioapic={} lapic0={:?} lapic1={:?} dev0={:?} dev1={:?}",
                restored_controller.page_exact(TWO_VCPU_CHECKPOINT_SHARED_PAGE),
                restored_controller.page_exact(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE),
                restored_controller.page_exact(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE),
                restored_controller.vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                restored_controller.vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                restored_controller.mp_state_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                restored_controller.mp_state_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                restored_controller.master_pic_exact(),
                restored_controller.slave_pic_exact(),
                restored_controller.ioapic_exact(),
                restored_controller.lapic_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                restored_controller.lapic_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                restored.device_exact(bars[0]),
                restored.device_exact(bars[1])
            );
            println!(
                "two-vCPU two-device capture rips: {:#x},{:#x}",
                result.first_capture_rip(),
                result.second_capture_rip()
            );
            println!(
                "two-vCPU two-device completion rips: {:#x},{:#x}",
                result.first_completion_rip(),
                result.second_completion_rip()
            );
            println!(
                "two-vCPU two-device proofs: {:?},{:?}",
                result.first_proof(),
                result.second_proof()
            );
            assert_eq!(TWO_VCPU_CHECKPOINT_OWNERSHIP_SET.len(), 3);
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
