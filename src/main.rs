use mini_hypervisor::config::VmConfig;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::loader::elf64::{
    expected_proof as expected_elf64_proof, proof_terminal_rip as elf64_terminal_rip,
    run_elf64_guest,
};
use mini_hypervisor::mmio_fixture::{
    run_mmio_guest, MMIO_GUEST_PROOF, MMIO_GUEST_TERMINAL_RIP, MMIO_GUEST_WRITE_VALUE,
};
use mini_hypervisor::vcpu::VcpuExit;
use mini_hypervisor::{
    run_cpuid_guest, run_debug_port_guest, run_hlt_guest, run_long_mode_guest,
    run_state_snapshot_roundtrip, verify_kvm_lifecycle,
};
use std::process::ExitCode;

const LONG_MODE_EXPECTED_PROOF: &[u8] = b"LM64";
const LONG_MODE_EXPECTED_TERMINAL_RIP: u64 = 0x1_0024;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

fn main() -> ExitCode {
    match run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("error: {error}");
            for source in error_sources(&error) {
                eprintln!("caused by: {source}");
            }
            ExitCode::FAILURE
        }
    }
}

fn error_sources(error: &(dyn std::error::Error + 'static)) -> Vec<String> {
    let mut sources = Vec::new();
    let mut current = error.source();
    while let Some(source) = current {
        sources.push(source.to_string());
        current = source.source();
    }
    sources
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
            let report = result.report();
            println!("long-mode proof: {:?}", result.proof());
            println!("{report}");

            if long_mode_proof_is_valid(
                result.proof(),
                report.exit(),
                report.rip(),
                report.rflags(),
            ) {
                Ok(ExitCode::SUCCESS)
            } else {
                eprintln!("long-mode deterministic proof contract failed");
                Ok(ExitCode::FAILURE)
            }
        }
        Some("run-elf64") => {
            let result = run_elf64_guest(VmConfig::default())?;
            let report = result.report();
            println!("elf64 proof: {:?}", result.proof());
            println!("{report}");

            if elf64_proof_is_valid(result.proof(), report.exit(), report.rip(), report.rflags()) {
                Ok(ExitCode::SUCCESS)
            } else {
                eprintln!("ELF64 deterministic execution proof contract failed");
                Ok(ExitCode::FAILURE)
            }
        }
        Some("run-mmio") => {
            let result = run_mmio_guest(VmConfig::default())?;
            let report = result.report();
            println!("mmio writes: {:?}", result.writes());
            println!("mmio proof: {:?}", result.proof());
            println!("{report}");

            if mmio_proof_is_valid(
                result.proof(),
                result.writes(),
                report.exit(),
                report.rip(),
                report.rflags(),
            ) {
                Ok(ExitCode::SUCCESS)
            } else {
                eprintln!("MMIO deterministic execution proof contract failed");
                Ok(ExitCode::FAILURE)
            }
        }
        Some(other) => {
            eprintln!(
                "usage: mini-hypervisor [probe|lifecycle|state-roundtrip|run-cpuid|run-hlt|run-debug-port|run-long-mode|run-elf64|run-mmio]"
            );
            eprintln!("unknown command: {other}");
            Ok(ExitCode::from(2))
        }
    }
}

fn terminal_proof_is_valid(
    proof: &[u8],
    expected_proof: &[u8],
    exit: VcpuExit,
    rip: u64,
    expected_rip: u64,
    rflags: u64,
) -> bool {
    proof == expected_proof
        && exit == VcpuExit::Hlt
        && rip == expected_rip
        && rflags & X86_RFLAGS_RESERVED_BIT == X86_RFLAGS_RESERVED_BIT
}

fn long_mode_proof_is_valid(proof: &[u8], exit: VcpuExit, rip: u64, rflags: u64) -> bool {
    terminal_proof_is_valid(
        proof,
        LONG_MODE_EXPECTED_PROOF,
        exit,
        rip,
        LONG_MODE_EXPECTED_TERMINAL_RIP,
        rflags,
    )
}

fn elf64_proof_is_valid(proof: &[u8], exit: VcpuExit, rip: u64, rflags: u64) -> bool {
    terminal_proof_is_valid(
        proof,
        expected_elf64_proof(),
        exit,
        rip,
        elf64_terminal_rip(),
        rflags,
    )
}

fn mmio_proof_is_valid(
    proof: &[u8],
    writes: &[u8],
    exit: VcpuExit,
    rip: u64,
    rflags: u64,
) -> bool {
    writes == [MMIO_GUEST_WRITE_VALUE]
        && terminal_proof_is_valid(
            proof,
            MMIO_GUEST_PROOF,
            exit,
            rip,
            MMIO_GUEST_TERMINAL_RIP,
            rflags,
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mini_hypervisor::error::{Error, HostEnvironmentError};
    use std::io;

    #[test]
    fn cli_diagnostics_preserve_operation_and_underlying_io_cause() {
        let error = Error::HostEnvironment(HostEnvironmentError::Io {
            operation: "KVM_GET_API_VERSION",
            source: io::Error::other("synthetic ioctl failure"),
        });

        assert_eq!(
            error.to_string(),
            "host I/O failure during KVM_GET_API_VERSION"
        );
        assert_eq!(
            error_sources(&error),
            vec!["synthetic ioctl failure".to_string()]
        );
    }

    #[test]
    fn long_mode_cli_proof_contract_requires_exact_proof_hlt_rip_and_reserved_rflags_bit() {
        assert!(long_mode_proof_is_valid(
            b"LM64",
            VcpuExit::Hlt,
            LONG_MODE_EXPECTED_TERMINAL_RIP,
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(long_mode_proof_is_valid(
            b"LM64",
            VcpuExit::Hlt,
            LONG_MODE_EXPECTED_TERMINAL_RIP,
            X86_RFLAGS_RESERVED_BIT | (1 << 4),
        ));
        assert!(!long_mode_proof_is_valid(
            b"LM6?",
            VcpuExit::Hlt,
            LONG_MODE_EXPECTED_TERMINAL_RIP,
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!long_mode_proof_is_valid(
            b"LM64",
            VcpuExit::Shutdown,
            LONG_MODE_EXPECTED_TERMINAL_RIP,
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!long_mode_proof_is_valid(
            b"LM64",
            VcpuExit::Hlt,
            LONG_MODE_EXPECTED_TERMINAL_RIP - 1,
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!long_mode_proof_is_valid(
            b"LM64",
            VcpuExit::Hlt,
            LONG_MODE_EXPECTED_TERMINAL_RIP,
            0,
        ));
    }

    #[test]
    fn elf64_cli_proof_contract_requires_exact_proof_hlt_rip_and_reserved_rflags_bit() {
        assert!(elf64_proof_is_valid(
            expected_elf64_proof(),
            VcpuExit::Hlt,
            elf64_terminal_rip(),
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!elf64_proof_is_valid(
            b"LM6?",
            VcpuExit::Hlt,
            elf64_terminal_rip(),
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!elf64_proof_is_valid(
            expected_elf64_proof(),
            VcpuExit::Shutdown,
            elf64_terminal_rip(),
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!elf64_proof_is_valid(
            expected_elf64_proof(),
            VcpuExit::Hlt,
            elf64_terminal_rip() - 1,
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!elf64_proof_is_valid(
            expected_elf64_proof(),
            VcpuExit::Hlt,
            elf64_terminal_rip(),
            0,
        ));
    }

    #[test]
    fn mmio_cli_proof_requires_write_readback_output_hlt_rip_and_reserved_rflags_bit() {
        assert!(mmio_proof_is_valid(
            MMIO_GUEST_PROOF,
            &[MMIO_GUEST_WRITE_VALUE],
            VcpuExit::Hlt,
            MMIO_GUEST_TERMINAL_RIP,
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!mmio_proof_is_valid(
            b"?MIO",
            &[MMIO_GUEST_WRITE_VALUE],
            VcpuExit::Hlt,
            MMIO_GUEST_TERMINAL_RIP,
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!mmio_proof_is_valid(
            MMIO_GUEST_PROOF,
            b"?",
            VcpuExit::Hlt,
            MMIO_GUEST_TERMINAL_RIP,
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!mmio_proof_is_valid(
            MMIO_GUEST_PROOF,
            &[MMIO_GUEST_WRITE_VALUE],
            VcpuExit::Shutdown,
            MMIO_GUEST_TERMINAL_RIP,
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!mmio_proof_is_valid(
            MMIO_GUEST_PROOF,
            &[MMIO_GUEST_WRITE_VALUE],
            VcpuExit::Hlt,
            MMIO_GUEST_TERMINAL_RIP - 1,
            X86_RFLAGS_RESERVED_BIT,
        ));
        assert!(!mmio_proof_is_valid(
            MMIO_GUEST_PROOF,
            &[MMIO_GUEST_WRITE_VALUE],
            VcpuExit::Hlt,
            MMIO_GUEST_TERMINAL_RIP,
            0,
        ));
    }
}
