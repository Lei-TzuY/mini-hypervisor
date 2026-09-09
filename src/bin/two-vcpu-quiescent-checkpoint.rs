use mini_hypervisor::config::VmConfig;
use mini_hypervisor::state_snapshot::{
    run_two_vcpu_quiescent_checkpoint_guest, TWO_VCPU_CHECKPOINT_FIRST_ID,
    TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE, TWO_VCPU_CHECKPOINT_SECOND_ID,
    TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE, TWO_VCPU_CHECKPOINT_SHARED_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_quiescent_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            println!("two-vCPU checkpoint BSP capture: {}", result.first_capture());
            println!("two-vCPU checkpoint AP capture: {}", result.second_capture());
            println!(
                "two-vCPU checkpoint pages: {:#x},{:#x},{:#x}",
                TWO_VCPU_CHECKPOINT_SHARED_PAGE.get(),
                TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE.get(),
                TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE.get()
            );
            println!(
                "two-vCPU checkpoint corruption: shared={} bsp_stack={} ap_stack={} bsp_vcpu={} ap_vcpu={}",
                result
                    .corruption()
                    .page_exact(TWO_VCPU_CHECKPOINT_SHARED_PAGE)
                    .unwrap_or(false),
                result
                    .corruption()
                    .page_exact(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE)
                    .unwrap_or(false),
                result
                    .corruption()
                    .page_exact(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE)
                    .unwrap_or(false),
                result
                    .corruption()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID)
                    .unwrap_or(false),
                result
                    .corruption()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
                    .unwrap_or(false)
            );
            println!(
                "two-vCPU checkpoint restore: shared={} bsp_stack={} ap_stack={} bsp_vcpu={} ap_vcpu={}",
                result
                    .restored()
                    .page_exact(TWO_VCPU_CHECKPOINT_SHARED_PAGE)
                    .unwrap_or(false),
                result
                    .restored()
                    .page_exact(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE)
                    .unwrap_or(false),
                result
                    .restored()
                    .page_exact(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE)
                    .unwrap_or(false),
                result
                    .restored()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID)
                    .unwrap_or(false),
                result
                    .restored()
                    .vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
                    .unwrap_or(false)
            );
            println!("two-vCPU checkpoint BSP proof: {:?}", result.first_proof());
            println!("two-vCPU checkpoint AP proof: {:?}", result.second_proof());
            println!("two-vCPU checkpoint BSP terminal: {}", result.first_terminal());
            println!("two-vCPU checkpoint AP terminal: {}", result.second_terminal());
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
