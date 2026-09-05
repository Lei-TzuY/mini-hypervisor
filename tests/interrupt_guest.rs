use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::{
    run_long_mode_interrupt_guest, LONG_MODE_INTERRUPT_PROOF, LONG_MODE_INTERRUPT_TERMINAL_RIP,
    LONG_MODE_INTERRUPT_VECTOR, X86_RFLAGS_INTERRUPT_ENABLE,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit, VcpuId};

#[test]
fn queued_direct_interrupt_enters_handler_iretq_resumes_guest_and_halts() {
    match run_long_mode_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.vector(), LONG_MODE_INTERRUPT_VECTOR);
            assert_eq!(result.proof(), LONG_MODE_INTERRUPT_PROOF);
            assert_eq!(result.io_exits().len(), LONG_MODE_INTERRUPT_PROOF.len());

            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(LONG_MODE_INTERRUPT_PROOF.iter().copied())
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
            assert_eq!(report.rip(), LONG_MODE_INTERRUPT_TERMINAL_RIP);
            assert_eq!(report.rflags() & 0x2, 0x2);
            assert_eq!(
                report.rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping direct-interrupt integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("direct interrupt guest execution failed unexpectedly: {error}"),
    }
}
