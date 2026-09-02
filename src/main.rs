use mini_hypervisor::config::VmConfig;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::{run_hlt_guest, verify_kvm_lifecycle};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), mini_hypervisor::error::Error> {
    match std::env::args().nth(1).as_deref() {
        Some("probe") | None => {
            let backend = KvmBackend::open()?;
            let capabilities = backend.capabilities();
            println!("KVM API version: {}", capabilities.api_version);
            println!("vCPU mmap size: {}", capabilities.vcpu_mmap_size);
            for capability in &capabilities.extensions {
                println!("{}: {}", capability.name, capability.value);
            }
            Ok(())
        }
        Some("lifecycle") => verify_kvm_lifecycle(VmConfig::default()),
        Some("run-hlt") => {
            let result = run_hlt_guest(VmConfig::default())?;
            println!("exit: {:?}", result.exit);
            println!("rip: {:#x}", result.rip);
            println!("rflags: {:#x}", result.rflags);
            Ok(())
        }
        Some(other) => {
            eprintln!("usage: mini-hypervisor [probe|lifecycle|run-hlt]");
            eprintln!("unknown command: {other}");
            Ok(())
        }
    }
}
