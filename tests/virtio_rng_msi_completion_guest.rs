use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::pci::virtio::{VIRTIO_F_VERSION_1, VIRTIO_RNG_TEST_PAYLOAD};
use mini_hypervisor::portio::virtio_rng_completion_interrupt_fixture::VIRTIO_RNG_INTERRUPT_BAR0_GPA;
use mini_hypervisor::portio::virtio_rng_msi_completion_fixture::{
    run_virtio_rng_msi_completion_guest, VIRTIO_RNG_MSI_ADDRESS, VIRTIO_RNG_MSI_DATA,
    VIRTIO_RNG_MSI_PROOF, VIRTIO_RNG_MSI_VECTOR,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{MmioDirection, PortIoDirection};

const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;

#[test]
fn virtio_rng_completion_uses_guest_programmed_msi_and_clears_isr() {
    match run_virtio_rng_msi_completion_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.msi_address(), VIRTIO_RNG_MSI_ADDRESS);
            assert_eq!(result.msi_data(), VIRTIO_RNG_MSI_DATA);
            assert_eq!(result.vector(), VIRTIO_RNG_MSI_VECTOR);
            assert_eq!(result.msi_delivery_count(), 1);
            assert_eq!(result.driver_features(), VIRTIO_F_VERSION_1);
            assert!(result.queue_enabled());
            assert_eq!(result.completion().descriptor_id(), 0);
            assert_eq!(
                result.completion().length(),
                VIRTIO_RNG_TEST_PAYLOAD.len() as u32
            );
            assert_eq!(result.used_idx(), 1);
            assert_eq!(result.used_id(), 0);
            assert_eq!(result.used_len(), VIRTIO_RNG_TEST_PAYLOAD.len() as u32);
            assert_eq!(result.payload(), VIRTIO_RNG_TEST_PAYLOAD);
            assert_eq!(result.proof(), VIRTIO_RNG_MSI_PROOF);

            assert_eq!(result.io_exits().len(), 27);
            for (exit, expected) in result.io_exits()[20..]
                .iter()
                .zip(VIRTIO_RNG_MSI_PROOF.iter().copied())
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
            assert_eq!(result.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping virtio-rng MSI completion integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("virtio-rng MSI completion execution failed unexpectedly: {error}"),
    }
}
