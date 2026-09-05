use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit, VcpuId};

#[test]
fn in_kernel_irqchip_routes_gsi0_through_pic_handler_and_resumes_guest() {
    match KvmBackend::run_irqchip_gsi_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.gsi(), KvmBackend::IRQCHIP_GSI);
            assert_eq!(result.vector(), KvmBackend::IRQCHIP_VECTOR);
            assert_eq!(result.armed_rflags() & 0x2, 0x2);
            assert_eq!(
                result.armed_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
            assert_eq!(result.proof(), KvmBackend::IRQCHIP_PROOF);
            assert_eq!(result.io_exits().len(), KvmBackend::IRQCHIP_PROOF.len());

            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(KvmBackend::IRQCHIP_PROOF.iter().copied())
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
            assert_eq!(report.rip(), KvmBackend::IRQCHIP_TERMINAL_RIP);
            assert_eq!(report.rflags() & 0x2, 0x2);
            assert_eq!(
                report.rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping irqchip GSI integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("irqchip GSI guest execution failed unexpectedly: {error}"),
    }
}
