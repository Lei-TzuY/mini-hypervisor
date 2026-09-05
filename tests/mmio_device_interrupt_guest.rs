use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::mmio::interrupt::{
    run_long_mode_mmio_interrupt_guest, MMIO_INTERRUPT_PROOF, MMIO_INTERRUPT_WRITE_VALUE,
};
use mini_hypervisor::mmio::long_mode::LONG_MODE_MMIO_DEVICE_GPA;
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{MmioDirection, PortIoDirection};

const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

#[test]
fn mmio_write_generates_one_device_event_and_controller_interrupt() {
    match run_long_mode_mmio_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.gsi(), KvmBackend::IRQCHIP_GSI);
            assert_eq!(result.vector(), KvmBackend::IRQCHIP_VECTOR);
            assert_eq!(result.device_event_count(), 1);
            assert_eq!(result.writes(), &[MMIO_INTERRUPT_WRITE_VALUE]);
            assert_eq!(result.proof(), MMIO_INTERRUPT_PROOF);

            let mmio = result.mmio_exit();
            assert_eq!(mmio.direction(), MmioDirection::Write);
            assert_eq!(mmio.address(), LONG_MODE_MMIO_DEVICE_GPA);
            assert_eq!(mmio.length(), 1);
            assert_eq!(mmio.write_data(), &[MMIO_INTERRUPT_WRITE_VALUE]);

            assert_eq!(
                result.lapic_spiv() & APIC_SPIV_SOFTWARE_ENABLE,
                APIC_SPIV_SOFTWARE_ENABLE
            );
            assert_eq!(
                result.lapic_lint0() & APIC_LVT_DELIVERY_MODE_MASK,
                APIC_LVT_DELIVERY_MODE_EXTINT
            );
            assert_eq!(result.lapic_lint0() & APIC_LVT_MASKED, 0);

            assert_eq!(result.armed_rflags() & 0x2, 0x2);
            assert_eq!(
                result.armed_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
            assert_eq!(result.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );

            assert_eq!(result.io_exits().len(), MMIO_INTERRUPT_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(MMIO_INTERRUPT_PROOF.iter().copied())
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
                "skipping MMIO device interrupt integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("MMIO device interrupt guest execution failed unexpectedly: {error}"),
    }
}
