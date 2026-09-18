use mini_hypervisor::state_snapshot::{
    run_versioned_full_controller_checkpoint_guest, CONTROLLER_CHECKPOINT_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_versioned_full_controller_checkpoint_guest() {
        Ok(result) => {
            let checkpoint = result.checkpoint();
            println!(
                "versioned full controller schema: version={} encoded_bytes={} pages={} msrs={} canonical={}",
                result.schema_version(),
                result.encoded_len(),
                result.page_count(),
                result.msr_count(),
                result.canonical_roundtrip()
            );
            println!(
                "versioned full controller capture: {}",
                checkpoint.capture()
            );
            println!(
                "versioned full controller page: {:#x}",
                CONTROLLER_CHECKPOINT_PAGE.get()
            );
            println!(
                "versioned full controller corruption: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={}",
                checkpoint
                    .corruption()
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                checkpoint.corruption().vcpu_exact(),
                checkpoint.corruption().master_pic_exact(),
                checkpoint.corruption().slave_pic_exact(),
                checkpoint.corruption().ioapic_exact(),
                checkpoint.corruption().lapic_exact()
            );
            println!(
                "versioned full controller restore: page={} vcpu={} master-pic={} slave-pic={} ioapic={} lapic={}",
                checkpoint
                    .restored()
                    .page_exact(CONTROLLER_CHECKPOINT_PAGE)
                    .unwrap_or(false),
                checkpoint.restored().vcpu_exact(),
                checkpoint.restored().master_pic_exact(),
                checkpoint.restored().slave_pic_exact(),
                checkpoint.restored().ioapic_exact(),
                checkpoint.restored().lapic_exact()
            );
            println!(
                "versioned full controller master PIC IMR: {:#x}",
                checkpoint.captured_master_pic_imr()
            );
            println!(
                "versioned full controller slave PIC IMR: {:#x}",
                checkpoint.captured_slave_pic_imr()
            );
            println!(
                "versioned full controller IOAPIC pin16: {:#x}",
                checkpoint.captured_ioapic_pin16()
            );
            println!(
                "versioned full controller LAPIC SPIV: {:#x}",
                checkpoint.captured_lapic_spiv()
            );
            println!(
                "versioned full controller LAPIC LINT0: {:#x}",
                checkpoint.captured_lapic_lint0()
            );
            println!(
                "versioned full controller slave armed rflags: {:#x}",
                checkpoint.slave_armed_rflags()
            );
            println!(
                "versioned full controller IOAPIC armed rflags: {:#x}",
                checkpoint.ioapic_armed_rflags()
            );
            println!(
                "versioned full controller completion rflags: {:#x}",
                checkpoint.completion_rflags()
            );
            println!("versioned full controller proof: {:?}", checkpoint.proof());
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
