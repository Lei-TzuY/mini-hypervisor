use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::pci::virtio::{
    VIRTIO_F_VERSION_1, VIRTIO_PCI_VENDOR_ID, VIRTIO_RNG_PCI_DEVICE_ID, VIRTIO_RNG_TEST_PAYLOAD,
    VIRTIO_STATUS_ACKNOWLEDGE, VIRTIO_STATUS_DRIVER, VIRTIO_STATUS_DRIVER_OK,
    VIRTIO_STATUS_FEATURES_OK,
};
use mini_hypervisor::portio::pci::{
    config_selector, PCI_CONFIG_ADDRESS_PORT, PCI_CONFIG_DATA_PORT,
};
use mini_hypervisor::portio::virtio_rng_fixture::{
    run_virtio_rng_pci_guest, VIRTIO_RNG_BAR0_GPA, VIRTIO_RNG_PROOF,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{MmioDirection, PortIoDirection, VcpuExit};

#[test]
fn modern_virtio_rng_discovers_negotiates_processes_one_split_request_and_halts() {
    match run_virtio_rng_pci_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(VIRTIO_PCI_VENDOR_ID, 0x1af4);
            assert_eq!(VIRTIO_RNG_PCI_DEVICE_ID, 0x1044);
            assert_eq!(VIRTIO_RNG_BAR0_GPA, 0x1000_0000);

            assert_eq!(result.driver_features(), VIRTIO_F_VERSION_1);
            assert_eq!(
                result.status(),
                VIRTIO_STATUS_ACKNOWLEDGE
                    | VIRTIO_STATUS_DRIVER
                    | VIRTIO_STATUS_FEATURES_OK
                    | VIRTIO_STATUS_DRIVER_OK
            );
            assert!(result.queue_enabled());
            assert_eq!(result.used_idx(), 1);
            assert_eq!(result.used_id(), 0);
            assert_eq!(result.used_len(), VIRTIO_RNG_TEST_PAYLOAD.len() as u32);
            assert_eq!(result.payload(), VIRTIO_RNG_TEST_PAYLOAD);
            assert_eq!(result.proof(), VIRTIO_RNG_PROOF);

            assert_eq!(result.io_exits().len(), 16);
            let selectors = [0x00, 0x34, 0x40, 0x50, 0x64, 0x10].map(config_selector);
            for (cycle, selector) in selectors.into_iter().enumerate() {
                let address = &result.io_exits()[cycle * 2];
                let data = &result.io_exits()[cycle * 2 + 1];
                assert_eq!(address.direction(), PortIoDirection::Out);
                assert_eq!(address.port(), PCI_CONFIG_ADDRESS_PORT);
                assert_eq!(address.size(), 4);
                assert_eq!(address.count(), 1);
                assert_eq!(address.output_data(), selector.to_le_bytes());
                assert_eq!(data.direction(), PortIoDirection::In);
                assert_eq!(data.port(), PCI_CONFIG_DATA_PORT);
                assert_eq!(data.size(), 4);
                assert_eq!(data.count(), 1);
                assert!(data.output_data().is_empty());
            }
            for (io, expected) in result.io_exits()[12..]
                .iter()
                .zip(VIRTIO_RNG_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.size(), 1);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            assert_eq!(result.mmio_exits().len(), 19);
            let notify = result.mmio_exits().last().unwrap();
            assert_eq!(notify.address(), VIRTIO_RNG_BAR0_GPA + 0x100);
            assert_eq!(notify.direction(), MmioDirection::Write);
            assert_eq!(notify.length(), 2);
            assert_eq!(notify.write_data(), &0_u16.to_le_bytes());

            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rip(), result.terminal_rip());
            assert_eq!(result.report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping virtio-rng PCI integration assertion: /dev/kvm is unavailable to this runner");
        }
        Err(error) => panic!("virtio-rng PCI request failed unexpectedly: {error}"),
    }
}
