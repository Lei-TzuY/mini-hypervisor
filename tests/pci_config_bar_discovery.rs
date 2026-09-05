use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::pci::{
    config_selector, PCI_CONFIG_ADDRESS_PORT, PCI_CONFIG_DATA_PORT, SYNTHETIC_PCI_BUS,
    SYNTHETIC_PCI_CLASS_CODE, SYNTHETIC_PCI_DEVICE, SYNTHETIC_PCI_DEVICE_ID,
    SYNTHETIC_PCI_FUNCTION, SYNTHETIC_PCI_VENDOR_ID,
};
use mini_hypervisor::portio::pci_fixture::{
    run_pci_discovery_guest, PCI_BAR0_GPA, PCI_BAR_WRITE_VALUE, PCI_DISCOVERY_PROOF,
    PCI_DISCOVERY_TERMINAL_RIP,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{MmioDirection, PortIoDirection, VcpuExit};

#[test]
fn pci_config_discovers_bar_and_drives_mmio_device() {
    match run_pci_discovery_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(SYNTHETIC_PCI_BUS, 0);
            assert_eq!(SYNTHETIC_PCI_DEVICE, 1);
            assert_eq!(SYNTHETIC_PCI_FUNCTION, 0);
            assert_eq!(SYNTHETIC_PCI_VENDOR_ID, 0xcafe);
            assert_eq!(SYNTHETIC_PCI_DEVICE_ID, 1);
            assert_eq!(SYNTHETIC_PCI_CLASS_CODE, 0xff);
            assert_eq!(PCI_BAR0_GPA, 0x1000_0000);

            assert_eq!(result.proof(), PCI_DISCOVERY_PROOF);
            assert_eq!(result.writes(), &[PCI_BAR_WRITE_VALUE]);
            assert_eq!(result.io_exits().len(), 10);
            assert_eq!(result.mmio_exits().len(), 1);

            let selectors = [
                config_selector(0x00),
                config_selector(0x08),
                config_selector(0x10),
            ];
            for (cycle, selector) in selectors.into_iter().enumerate() {
                let base = cycle * 3;
                let address = &result.io_exits()[base];
                assert_eq!(address.direction(), PortIoDirection::Out);
                assert_eq!(address.port(), PCI_CONFIG_ADDRESS_PORT);
                assert_eq!(address.size(), 4);
                assert_eq!(address.count(), 1);
                assert_eq!(address.output_data(), selector.to_le_bytes());

                let data = &result.io_exits()[base + 1];
                assert_eq!(data.direction(), PortIoDirection::In);
                assert_eq!(data.port(), PCI_CONFIG_DATA_PORT);
                assert_eq!(data.size(), 4);
                assert_eq!(data.count(), 1);
                assert!(data.output_data().is_empty());

                let proof = &result.io_exits()[base + 2];
                assert_eq!(proof.direction(), PortIoDirection::Out);
                assert_eq!(proof.port(), DEBUG_PORT);
                assert_eq!(proof.size(), 1);
                assert_eq!(proof.count(), 1);
                assert_eq!(proof.output_data(), &[PCI_DISCOVERY_PROOF[cycle]]);
            }

            let completion = &result.io_exits()[9];
            assert_eq!(completion.direction(), PortIoDirection::Out);
            assert_eq!(completion.port(), DEBUG_PORT);
            assert_eq!(completion.output_data(), &[PCI_DISCOVERY_PROOF[3]]);

            let mmio = &result.mmio_exits()[0];
            assert_eq!(mmio.address(), PCI_BAR0_GPA);
            assert_eq!(mmio.direction(), MmioDirection::Write);
            assert_eq!(mmio.length(), 1);
            assert_eq!(mmio.write_data(), &[PCI_BAR_WRITE_VALUE]);

            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rip(), PCI_DISCOVERY_TERMINAL_RIP);
            assert_eq!(result.report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping PCI configuration BAR discovery assertion: /dev/kvm is unavailable to this runner");
        }
        Err(error) => panic!("PCI configuration BAR discovery failed unexpectedly: {error}"),
    }
}
