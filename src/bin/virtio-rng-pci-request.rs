use mini_hypervisor::config::VmConfig;
use mini_hypervisor::portio::pci::virtio::{
    VIRTIO_F_VERSION_1, VIRTIO_PCI_VENDOR_ID, VIRTIO_RNG_PCI_DEVICE_ID,
};
use mini_hypervisor::portio::virtio_rng_fixture::{
    run_virtio_rng_pci_guest, VIRTIO_RNG_BAR0_GPA, VIRTIO_RNG_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_virtio_rng_pci_guest(VmConfig::default()) {
        Ok(result) => {
            println!(
                "virtio-rng PCI identity: vendor={:#06x} device={:#06x}",
                VIRTIO_PCI_VENDOR_ID, VIRTIO_RNG_PCI_DEVICE_ID
            );
            println!("virtio-rng BAR0: {VIRTIO_RNG_BAR0_GPA:#x}");
            println!("virtio-rng driver features: {:#x}", result.driver_features());
            println!("virtio-rng device status: {:#04x}", result.status());
            println!("virtio-rng queue enabled: {}", result.queue_enabled());
            println!(
                "virtio-rng used: idx={} id={} len={}",
                result.used_idx(),
                result.used_id(),
                result.used_len()
            );
            println!("virtio-rng payload: {:?}", result.payload());
            println!("virtio-rng proof: {:?}", result.proof());
            println!("virtio-rng port-I/O exits: {}", result.io_exits().len());
            println!("virtio-rng MMIO exits: {}", result.mmio_exits().len());
            println!("{}", result.report());

            if result.driver_features() == VIRTIO_F_VERSION_1
                && result.queue_enabled()
                && result.proof() == VIRTIO_RNG_PROOF
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: virtio-rng PCI request proof did not match expected state");
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
