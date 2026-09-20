use mini_hypervisor::state_snapshot::{
    run_two_vcpu_full_controller_checkpoint_guest, TWO_VCPU_CHECKPOINT_FIRST_ID,
    TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE, TWO_VCPU_CHECKPOINT_OWNERSHIP_SET,
    TWO_VCPU_CHECKPOINT_SECOND_ID, TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
    TWO_VCPU_CHECKPOINT_SHARED_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_full_controller_checkpoint_guest() {
        Ok(result) => {
            println!(
                "two-vCPU full-controller pages: {}",
                result
                    .captured_pages()
                    .iter()
                    .map(|address| format!("{:#x}", address.get()))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            println!(
                "two-vCPU full-controller corruption: shared={:?} first-stack={:?} second-stack={:?} vcpu0={:?} vcpu1={:?} mp0={:?} mp1={:?} master={} slave={} ioapic={} lapic0={:?} lapic1={:?}",
                result.corruption().page_exact(TWO_VCPU_CHECKPOINT_SHARED_PAGE),
                result
                    .corruption()
                    .page_exact(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE),
                result
                    .corruption()
                    .page_exact(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE),
                result.corruption().vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                result.corruption().vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                result.corruption().mp_state_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                result.corruption().mp_state_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                result.corruption().master_pic_exact(),
                result.corruption().slave_pic_exact(),
                result.corruption().ioapic_exact(),
                result.corruption().lapic_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                result.corruption().lapic_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
            );
            println!(
                "two-vCPU full-controller restore: shared={:?} first-stack={:?} second-stack={:?} vcpu0={:?} vcpu1={:?} master={} slave={} ioapic={} lapic0={:?} lapic1={:?}",
                result.restored().page_exact(TWO_VCPU_CHECKPOINT_SHARED_PAGE),
                result
                    .restored()
                    .page_exact(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE),
                result
                    .restored()
                    .page_exact(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE),
                result.restored().vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                result.restored().vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                result.restored().mp_state_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                result.restored().mp_state_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                result.restored().master_pic_exact(),
                result.restored().slave_pic_exact(),
                result.restored().ioapic_exact(),
                result.restored().lapic_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                result.restored().lapic_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
            );
            println!(
                "two-vCPU full-controller first capture: rip={:#x} rflags={:#x}",
                result.first_capture_rip(),
                result.first_capture_rflags()
            );
            println!(
                "two-vCPU full-controller second capture: rip={:#x} rflags={:#x}",
                result.second_capture_rip(),
                result.second_capture_rflags()
            );
            println!(
                "two-vCPU full-controller first proof: {:?}",
                result.first_proof()
            );
            println!(
                "two-vCPU full-controller second proof: {:?}",
                result.second_proof()
            );
            println!(
                "two-vCPU full-controller first completion: rip={:#x} rflags={:#x}",
                result.first_completion_rip(),
                result.first_completion_rflags()
            );
            println!(
                "two-vCPU full-controller second completion: rip={:#x} rflags={:#x}",
                result.second_completion_rip(),
                result.second_completion_rflags()
            );
            assert_eq!(result.captured_pages(), TWO_VCPU_CHECKPOINT_OWNERSHIP_SET);
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
