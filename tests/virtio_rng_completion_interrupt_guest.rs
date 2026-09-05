use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::pci::virtio::{VIRTIO_F_VERSION_1, VIRTIO_RNG_TEST_PAYLOAD};
use mini_hypervisor::portio::virtio_rng_completion_interrupt_fixture::{
    run_virtio_rng_completion_interrupt_guest, VIRTIO_RNG_INTERRUPT_BAR0_GPA,
    VIRTIO_RNG_INTERRUPT_PROOF,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{MmioDirection, PortIoDirection, VcpuExit};

const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

#[test]
fn virtio_rng_completion_asserts_intx_reads_isr_and_deasserts_before_resume() {
    match run_virtio_rng_completion_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.gsi(), 0);
            assert_eq!(result.vector(), 0x40);
            assert_eq!(result.assert_count(), 1);
            assert_eq!(result.deassert_count(), 1);
            assert_eq!(result.driver_features(), VIRTIO_F_VERSION_1);
            assert!(result.queue_enabled());
            assert_eq!(result.completion().descriptor_id(), 0);
            assert_eq!(result.completion().length(), VIRTIO_RNG_TEST_PAYLOAD.len() as u32);
            assert_eq!(result.used_idx(), 1);
            assert_eq!(result.used_id(), 0);
            assert_eq!(result.used_len(), VIRTIO_RNG_TEST_PAYLOAD.len() as u32);
            assert_eq!(result.payload(), VIRTIO_RNG_TEST_PAYLOAD);
            assert_eq!(result.proof(), VIRTIO_RNG_INTERRUPT_PROOF);

            assert_eq!(result.io_exits().len(), VIRTIO_RNG_INTERRUPT_PROOF.len() + 12);
            for (exit, expected) in result.io_exits()[12..]
                .iter()
                .zip(VIRTIO_RNG_INTERRUPT_PROOF.iter().copied())
            {
                assert_eq!(exit.direction(), PortIoDirection::Out);
                assert_eq!(exit.port(), DEBUG_PORT);
                assert_eq!(exit.size(), 1);
                assert_eq!(exit.count(), 1);
                assert_eq!(exit.output_data(), &[expected]);
            }

            assert_eq!(result.mmio_exits().len(), 21);
            let first_isr = &result.mmio_exits()[19];
            let second_isr = &result.mmio_exits()[20];
            for exit in [first_isr, second_isr] {
                assert_eq!(exit.address(), VIRTIO_RNG_INTERRUPT_BAR0_GPA + 0x200);
                assert_eq!(exit.direction(), MmioDirection::Read);
                assert_eq!(exit.length(), 1);
                assert!(exit.write_data().is_empty());
            }

            assert_eq!(
                result.lapic_spiv() & APIC_SPIV_SOFTWARE_ENABLE,
                APIC_SPIV_SOFTWARE_ENABLE
            );
            assert_eq!(
                result.lapic_lint0() & APIC_LVT_DELIVERY_MODE_MASK,
                APIC_LVT_DELIVERY_MODE_EXTINT
            );
            assert_eq!(result.lapic_lint0() & APIC_LVT_MASKED, 0);

            let report = result.report();
            assert_eq!(report.exit(), VcpuExit::Hlt);
            assert_eq!(report.rip(), result.terminal_rip());
            assert_eq!(report.rflags() & 0x2, 0x2);
            assert_eq!(
                report.rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping virtio-rng completion interrupt integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("virtio-rng completion interrupt execution failed unexpectedly: {error}"),
    }
}
