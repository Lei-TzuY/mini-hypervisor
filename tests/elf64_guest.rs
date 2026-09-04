use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::loader::elf64::{expected_proof, proof_terminal_rip, run_elf64_guest};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit, VcpuId};

#[test]
fn deterministic_elf64_image_loads_executes_in_long_mode_and_halts() {
    match run_elf64_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.proof(), expected_proof());
            assert_eq!(result.io_exits().len(), expected_proof().len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(expected_proof().iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            let report = result.report();
            assert_eq!(report.vcpu_id(), VcpuId::BOOT);
            assert_eq!(report.exit(), VcpuExit::Hlt);
            assert_eq!(report.rip(), proof_terminal_rip());
            assert_eq!(report.rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping ELF64 execution integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("ELF64 guest execution failed unexpectedly: {error}"),
    }
}
