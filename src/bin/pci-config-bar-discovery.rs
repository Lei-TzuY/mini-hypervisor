use mini_hypervisor::config::VmConfig;
use mini_hypervisor::portio::pci::{
    SYNTHETIC_PCI_BUS, SYNTHETIC_PCI_CLASS_CODE, SYNTHETIC_PCI_DEVICE, SYNTHETIC_PCI_DEVICE_ID,
    SYNTHETIC_PCI_FUNCTION, SYNTHETIC_PCI_VENDOR_ID,
};
use mini_hypervisor::portio::pci_fixture::{
    run_pci_discovery_guest, PCI_BAR0_GPA, PCI_DISCOVERY_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_pci_discovery_guest(VmConfig::default()) {
        Ok(result) => {
            println!(
                "pci function: {:02x}:{:02x}.{}",
                SYNTHETIC_PCI_BUS, SYNTHETIC_PCI_DEVICE, SYNTHETIC_PCI_FUNCTION
            );
            println!(
                "pci identity: vendor={:#06x} device={:#06x}",
                SYNTHETIC_PCI_VENDOR_ID, SYNTHETIC_PCI_DEVICE_ID
            );
            println!("pci class: {:#04x}", SYNTHETIC_PCI_CLASS_CODE);
            println!("pci BAR0: {PCI_BAR0_GPA:#x}");
            println!("pci BAR writes: {:?}", result.writes());
            println!("pci config/MMIO proof: {:?}", result.proof());
            println!("pci port-I/O exits: {}", result.io_exits().len());
            println!("pci MMIO exits: {}", result.mmio_exits().len());
            println!("{}", result.report());

            if result.proof() == PCI_DISCOVERY_PROOF {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: PCI discovery proof did not match expected bytes");
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("error: {error}");
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                eprintln!("caused by: {cause}");
                source = cause.source();
            }
            ExitCode::FAILURE
        }
    }
}
