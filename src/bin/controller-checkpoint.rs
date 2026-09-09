use mini_hypervisor::state_snapshot::{
    run_controller_checkpoint_guest, CONTROLLER_CHECKPOINT_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_controller_checkpoint_guest() {
        Ok(result) => {
            println!("controller checkpoint capture: {}", result.capture());
            println!(
                "controller checkpoint page: {:#x}",
                CONTROLLER_CHECKPOINT_PAGE.get()
            );
            println!(
                "controller checkpoint corruption: page={} vcpu={} master-pic={} lapic={}",
                result
                    .corruption()
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                result.corruption().vcpu_exact(),
                result.corruption().master_pic_exact(),
                result.corruption().lapic_exact()
            );
            println!(
                "controller checkpoint restore: page={} vcpu={} master-pic={} lapic={}",
                result
                    .restored()
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                result.restored().vcpu_exact(),
                result.restored().master_pic_exact(),
                result.restored().lapic_exact()
            );
            println!(
                "controller checkpoint PIC IMR: {:#x}",
                result.captured_pic_imr()
            );
            println!(
                "controller checkpoint LAPIC SPIV: {:#x}",
                result.captured_lapic_spiv()
            );
            println!(
                "controller checkpoint LAPIC LINT0: {:#x}",
                result.captured_lapic_lint0()
            );
            println!(
                "controller checkpoint armed rflags: {:#x}",
                result.armed_rflags()
            );
            println!(
                "controller checkpoint completion rflags: {:#x}",
                result.completion_rflags()
            );
            println!("controller checkpoint proof: {:?}", result.proof());
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
