use mini_hypervisor::config::VmConfig;
use mini_hypervisor::kvm::KvmBackend;
use std::process::ExitCode;

fn main() -> ExitCode {
    match KvmBackend::run_irqchip_gsi_guest(VmConfig::default()) {
        Ok(result) => {
            println!("irqchip GSI: {}", result.gsi());
            println!("irqchip vector: {:#x}", result.vector());
            println!("irqchip LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("irqchip LAPIC LINT0: {:#x}", result.lapic_lint0());
            println!("irqchip armed rflags: {:#x}", result.armed_rflags());
            println!("irqchip proof: {:?}", result.proof());
            println!("{}", result.report());
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
