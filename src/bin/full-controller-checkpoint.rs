use mini_hypervisor::state_snapshot::{
    run_full_controller_checkpoint_guest, CONTROLLER_CHECKPOINT_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_full_controller_checkpoint_guest() {
        Ok(result) => {
            println!("full controller checkpoint capture: {}", result.capture());
            println!(
                "full controller checkpoint page: {:#x}",
                CONTROLLER_CHECKPOINT_PAGE.get()
            );
            println!(
                "full controller checkpoint corruption: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={}",
                result
                    .corruption()
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                result.corruption().vcpu_exact(),
                result.corruption().master_pic_exact(),
                result.corruption().slave_pic_exact(),
                result.corruption().ioapic_exact(),
                result.corruption().lapic_exact()
            );
            println!(
                "full controller checkpoint restore: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={}",
                result
                    .restored()
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                result.restored().vcpu_exact(),
                result.restored().master_pic_exact(),
                result.restored().slave_pic_exact(),
                result.restored().ioapic_exact(),
                result.restored().lapic_exact()
            );
            println!(
                "full controller checkpoint master PIC IMR: {:#x}",
                result.captured_master_pic_imr()
            );
            println!(
                "full controller checkpoint slave PIC IMR: {:#x}",
                result.captured_slave_pic_imr()
            );
            println!(
                "full controller checkpoint IOAPIC base: {:#x}",
                result.captured_ioapic_base()
            );
            println!(
                "full controller checkpoint IOAPIC pin16: {:#x}",
                result.captured_ioapic_pin16()
            );
            println!(
                "full controller checkpoint LAPIC SPIV: {:#x}",
                result.captured_lapic_spiv()
            );
            println!(
                "full controller checkpoint LAPIC LINT0: {:#x}",
                result.captured_lapic_lint0()
            );
            println!(
                "full controller checkpoint slave armed rflags: {:#x}",
                result.slave_armed_rflags()
            );
            println!(
                "full controller checkpoint IOAPIC armed rflags: {:#x}",
                result.ioapic_armed_rflags()
            );
            println!(
                "full controller checkpoint completion rflags: {:#x}",
                result.completion_rflags()
            );
            println!("full controller checkpoint proof: {:?}", result.proof());
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
