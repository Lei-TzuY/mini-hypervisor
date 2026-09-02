use mini_hypervisor::config::VmConfig;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::{run_debug_port_guest, run_hlt_guest, verify_kvm_lifecycle};
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
            let report = run_hlt_guest(VmConfig::default())?;
            println!("{report}");
            Ok(())
        }
        Some("run-debug-port") => {
            let result = run_debug_port_guest(VmConfig::default())?;
            let io = result.io();
            println!(
                "io: direction={:?}, size={}, port={:#x}, count={}, data={:?}",
                io.direction(),
                io.size(),
                io.port(),
                io.count(),
                io.output_data()
            );
            println!("debug output: {:?}", result.output());
            println!("{}", result.report());
            Ok(())
        }
        Some(other) => {
            eprintln!(
                "usage: mini-hypervisor [probe|lifecycle|run-hlt|run-debug-port]"
            );
            eprintln!("unknown command: {other}");
            Ok(())
        }
    }
}
