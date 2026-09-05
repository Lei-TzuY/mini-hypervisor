use mini_hypervisor::config::VmConfig;
use mini_hypervisor::kvm::KvmBackend;
use std::process::ExitCode;

fn main() -> ExitCode {
    match KvmBackend::run_async_timer_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            println!("async timer GSI: {}", result.gsi());
            println!("async timer vector: {:#x}", result.vector());
            println!("async timer LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("async timer LAPIC LINT0: {:#x}", result.lapic_lint0());
            println!("async timer armed rflags: {:#x}", result.armed_rflags());
            println!(
                "async timer completion rflags: {:#x}",
                result.completion_rflags()
            );
            println!("async timer proof: {:?}", result.proof());
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
