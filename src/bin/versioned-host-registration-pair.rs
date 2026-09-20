use mini_hypervisor::config::VmConfig;
use mini_hypervisor::kvm::KvmBackend;
use std::process::ExitCode;

fn main() -> ExitCode {
    match KvmBackend::run_versioned_two_host_registration_acceleration_guest(VmConfig::default()) {
        Ok(result) => {
            println!(
                "versioned registration-pair schema: version={} encoded_bytes={} nested_versions={:?} canonical={}",
                result.schema_version(),
                result.encoded_len(),
                result.registration_versions(),
                result.canonical_roundtrip()
            );
            let acceleration = result.acceleration();
            println!(
                "versioned registration-pair doorbells: {:?}",
                acceleration.doorbells()
            );
            println!(
                "versioned registration-pair gsis: {:?}",
                acceleration.gsis()
            );
            println!(
                "versioned registration-pair vectors: {:?}",
                acceleration.vectors()
            );
            println!(
                "versioned registration-pair generation events: {:?}",
                acceleration.generation_doorbell_events()
            );
            println!(
                "versioned registration-pair proof: {:?}",
                acceleration.proof()
            );
            println!(
                "versioned registration-pair completion rflags: {:#x}",
                acceleration.completion_rflags()
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
