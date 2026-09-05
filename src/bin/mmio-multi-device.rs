use mini_hypervisor::config::VmConfig;
use mini_hypervisor::mmio::multi_device::run_multi_device_mmio_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_multi_device_mmio_guest(VmConfig::default()) {
        Ok(result) => {
            println!("multi-device first writes: {:?}", result.first_writes());
            println!("multi-device second writes: {:?}", result.second_writes());
            println!("multi-device MMIO exits: {}", result.mmio_exits().len());
            println!("multi-device proof: {:?}", result.proof());
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
