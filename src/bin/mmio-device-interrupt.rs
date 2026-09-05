use mini_hypervisor::config::VmConfig;
use mini_hypervisor::mmio::interrupt::run_long_mode_mmio_interrupt_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_long_mode_mmio_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            println!("mmio interrupt GSI: {}", result.gsi());
            println!("mmio interrupt vector: {:#x}", result.vector());
            println!(
                "mmio interrupt device events: {}",
                result.device_event_count()
            );
            println!("mmio interrupt writes: {:?}", result.writes());
            println!("mmio interrupt LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("mmio interrupt LAPIC LINT0: {:#x}", result.lapic_lint0());
            println!("mmio interrupt armed rflags: {:#x}", result.armed_rflags());
            println!(
                "mmio interrupt completion rflags: {:#x}",
                result.completion_rflags()
            );
            println!("mmio interrupt proof: {:?}", result.proof());
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
