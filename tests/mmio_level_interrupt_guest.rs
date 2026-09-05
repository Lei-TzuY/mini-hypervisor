use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::mmio::level_interrupt::{
    run_long_mode_mmio_level_interrupt_guest, MMIO_LEVEL_INTERRUPT_ACK_VALUE,
    MMIO_LEVEL_INTERRUPT_COMMAND_VALUE, MMIO_LEVEL_INTERRUPT_PROOF,
};
use mini_hypervisor::mmio::long_mode::LONG_MODE_MMIO_DEVICE_GPA;
use mini_hypervisor::mmio::{LEVEL_INTERRUPT_ACK_OFFSET, LEVEL_INTERRUPT_STATUS_OFFSET};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{MmioDirection, PortIoDirection};

const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

#[test]
fn handler_status_ack_controls_level_interrupt_line_lifecycle() {
    match run_long_mode_mmio_level_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.gsi(), KvmBackend::IRQCHIP_GSI);
            assert_eq!(result.vector(), KvmBackend::IRQCHIP_VECTOR);
            assert_eq!(result.assert_event_count(), 1);
            assert_eq!(result.deassert_event_count(), 1);
            assert_eq!(
                result.writes(),
                &[
                    MMIO_LEVEL_INTERRUPT_COMMAND_VALUE,
                    MMIO_LEVEL_INTERRUPT_ACK_VALUE
                ]
            );
            assert_eq!(result.proof(), MMIO_LEVEL_INTERRUPT_PROOF);

            let command = result.command_exit();
            assert_eq!(command.direction(), MmioDirection::Write);
            assert_eq!(command.address(), LONG_MODE_MMIO_DEVICE_GPA);
            assert_eq!(command.length(), 1);
            assert_eq!(command.write_data(), &[MMIO_LEVEL_INTERRUPT_COMMAND_VALUE]);

            let status = result.status_exit();
            assert_eq!(status.direction(), MmioDirection::Read);
            assert_eq!(
                status.address(),
                LONG_MODE_MMIO_DEVICE_GPA + LEVEL_INTERRUPT_STATUS_OFFSET
            );
            assert_eq!(status.length(), 1);
            assert!(status.write_data().is_empty());

            let ack = result.ack_exit();
            assert_eq!(ack.direction(), MmioDirection::Write);
            assert_eq!(
                ack.address(),
                LONG_MODE_MMIO_DEVICE_GPA + LEVEL_INTERRUPT_ACK_OFFSET
            );
            assert_eq!(ack.length(), 1);
            assert_eq!(ack.write_data(), &[MMIO_LEVEL_INTERRUPT_ACK_VALUE]);

            assert_eq!(
                result.lapic_spiv() & APIC_SPIV_SOFTWARE_ENABLE,
                APIC_SPIV_SOFTWARE_ENABLE
            );
            assert_eq!(
                result.lapic_lint0() & APIC_LVT_DELIVERY_MODE_MASK,
                APIC_LVT_DELIVERY_MODE_EXTINT
            );
            assert_eq!(result.lapic_lint0() & APIC_LVT_MASKED, 0);

            for rflags in [result.armed_rflags(), result.completion_rflags()] {
                assert_eq!(rflags & 0x2, 0x2);
                assert_eq!(
                    rflags & X86_RFLAGS_INTERRUPT_ENABLE,
                    X86_RFLAGS_INTERRUPT_ENABLE
                );
            }

            assert_eq!(result.io_exits().len(), MMIO_LEVEL_INTERRUPT_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(MMIO_LEVEL_INTERRUPT_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping level interrupt lifecycle integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => {
            panic!("level interrupt lifecycle guest execution failed unexpectedly: {error}")
        }
    }
}
