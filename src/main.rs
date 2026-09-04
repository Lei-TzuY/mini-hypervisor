use mini_hypervisor::config::VmConfig;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::{
    run_cpuid_guest, run_debug_port_guest, run_hlt_guest, run_long_mode_guest,
    run_state_snapshot_roundtrip, verify_kvm_lifecycle,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, mini_hypervisor::error::Error> {
    match std::env::args().nth(1).as_deref() {
        Some("probe") | None => {
            let backend = KvmBackend::open()?;
            let capabilities = backend.capabilities();
            println!("KVM API version: {}", capabilities.api_version);
            println!("vCPU mmap size: {}", capabilities.vcpu_mmap_size);
            for capability in &capabilities.extensions {
                println!("{}: {}", capability.name, capability.value);
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("lifecycle") => {
            verify_kvm_lifecycle(VmConfig::default())?;
            Ok(ExitCode::SUCCESS)
        }
        Some("state-roundtrip") => {
            let result = run_state_snapshot_roundtrip(VmConfig::default())?;
            println!("changed exact: {}", result.changed().is_exact_match());
            println!("restored exact: {}", result.restored().is_exact_match());
            println!(
                "restored registers exact: {}",
                result.restored().registers().is_exact_match()
            );
            println!(
                "restored special registers exact: {}",
                result.restored().special_registers().is_exact_match()
            );
            println!(
                "restored MSRs exact: {}",
                result.restored().msrs().is_exact_match()
            );
            Ok(ExitCode::SUCCESS)
        }
        Some("run-cpuid") => {
            let result = run_cpuid_guest(VmConfig::default())?;
            println!("cpuid(1).ecx: {:#010x}", result.cpuid1_ecx());
            println!("cpuid(0x40000001).eax: {:#010x}", result.kvm_features_eax());
            println!(
                "masked LAPIC-dependent features clear: {}",
                result.masked_lapic_features_clear()
            );
            println!("{}", result.report());
            Ok(ExitCode::SUCCESS)
        }
        Some("run-hlt") => {
            let report = run_hlt_guest(VmConfig::default())?;
            println!("{report}");
            Ok(ExitCode::SUCCESS)
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
            Ok(ExitCode::SUCCESS)
        }
        Some("run-long-mode") => {
            let result = run_long_mode_guest(VmConfig::default())?;
            println!("long-mode proof: {:?}", result.proof());
            println!("{}", result.report());
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => {
            eprintln!(
                "usage: mini-hypervisor [probe|lifecycle|state-roundtrip|run-cpuid|run-hlt|run-debug-port|run-long-mode]"
            );
            eprintln!("unknown command: {other}");
            Ok(ExitCode::from(2))
        }
    }
}
