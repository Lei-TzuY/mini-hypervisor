use mini_hypervisor::config::VmConfig;
use mini_hypervisor::mmio::level_interrupt::run_long_mode_mmio_level_interrupt_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_long_mode_mmio_level_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            println!("level interrupt GSI: {}", result.gsi());
            println!("level interrupt vector: {:#x}", result.vector());
            println!(
                "level interrupt assert events: {}",
                result.assert_event_count()
            );
            println!(
                "level interrupt deassert events: {}",
                result.deassert_event_count()
            );
            println!("level interrupt writes: {:?}", result.writes());
            println!("level interrupt LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("level interrupt LAPIC LINT0: {:#x}", result.lapic_lint0());
            println!("level interrupt armed rflags: {:#x}", result.armed_rflags());
            println!(
                "level interrupt completion rflags: {:#x}",
                result.completion_rflags()
            );
            println!("level interrupt proof: {:?}", result.proof());
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
