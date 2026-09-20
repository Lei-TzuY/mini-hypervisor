use mini_hypervisor::config::VmConfig;
use mini_hypervisor::kvm::KvmBackend;
use std::process::ExitCode;

fn main() -> ExitCode {
    match KvmBackend::run_two_host_registration_acceleration_guest(VmConfig::default()) {
        Ok(result) => {
            println!(
                "two-registration doorbells: {:?}",
                result.doorbells()
            );
            println!("two-registration gsis: {:?}", result.gsis());
            println!("two-registration vectors: {:?}", result.vectors());
            println!(
                "two-registration generation events: {:?}",
                result.generation_doorbell_events()
            );
            println!("two-registration proof: {:?}", result.proof());
            println!(
                "two-registration completion rflags: {:#x}",
                result.completion_rflags()
            );
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
