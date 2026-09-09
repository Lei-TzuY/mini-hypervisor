use mini_hypervisor::state_snapshot::{
    run_two_vcpu_checkpoint_guest, TWO_VCPU_CHECKPOINT_FIRST_ID,
    TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE, TWO_VCPU_CHECKPOINT_SECOND_ID,
    TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE, TWO_VCPU_CHECKPOINT_SHARED_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_checkpoint_guest() {
        Ok(result) => {
            println!("two-vCPU checkpoint first capture: {}", result.first_capture());
            println!("two-vCPU checkpoint second capture: {}", result.second_capture());
            println!(
                "two-vCPU checkpoint pages: {:#x},{:#x},{:#x}",
                TWO_VCPU_CHECKPOINT_SHARED_PAGE.get(),
                TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE.get(),
                TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE.get()
            );
            println!(
                "two-vCPU checkpoint corruption: shared={} first-stack={} second-stack={} vcpu0={} vcpu1={}",
                result
                    .corruption()
                    .page_exact(TWO_VCPU_CHECKPOINT_SHARED_PAGE)
                    .expect("shared page belongs to the checkpoint"),
                result
                    .corruption()
                    .page_exact(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE)
                    .expect("first stack page belongs to the checkpoint"),
                result
                    .corruption()
                    .page_exact(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE)
                    .expect("second stack page belongs to the checkpoint"),
                result
                    .corruption()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID)
                    .expect("first vCPU belongs to the checkpoint"),
                result
                    .corruption()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
                    .expect("second vCPU belongs to the checkpoint")
            );
            println!(
                "two-vCPU checkpoint restore: shared={} first-stack={} second-stack={} vcpu0={} vcpu1={}",
                result
                    .restored()
                    .page_exact(TWO_VCPU_CHECKPOINT_SHARED_PAGE)
                    .expect("shared page belongs to the checkpoint"),
                result
                    .restored()
                    .page_exact(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE)
                    .expect("first stack page belongs to the checkpoint"),
                result
                    .restored()
                    .page_exact(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE)
                    .expect("second stack page belongs to the checkpoint"),
                result
                    .restored()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID)
                    .expect("first vCPU belongs to the checkpoint"),
                result
                    .restored()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
                    .expect("second vCPU belongs to the checkpoint")
            );
            println!("two-vCPU checkpoint first proof: {:?}", result.first_proof());
            println!("two-vCPU checkpoint second proof: {:?}", result.second_proof());
            println!("two-vCPU checkpoint first terminal: {}", result.first_terminal());
            println!("two-vCPU checkpoint second terminal: {}", result.second_terminal());
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
