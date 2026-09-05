use mini_hypervisor::config::VmConfig;
use mini_hypervisor::portio::pci::virtio::{VIRTIO_F_VERSION_1, VIRTIO_PCI_VENDOR_ID};
use mini_hypervisor::portio::pci::virtio_blk::{
    deterministic_sector, VIRTIO_BLK_CAPACITY_SECTORS, VIRTIO_BLK_PCI_DEVICE_ID, VIRTIO_BLK_S_OK,
};
use mini_hypervisor::portio::virtio_blk_fixture::{
    run_virtio_blk_pci_guest, VIRTIO_BLK_BAR0_GPA, VIRTIO_BLK_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_virtio_blk_pci_guest(VmConfig::default()) {
        Ok(result) => {
            println!(
                "virtio-blk PCI identity: vendor={:#06x} device={:#06x}",
                VIRTIO_PCI_VENDOR_ID, VIRTIO_BLK_PCI_DEVICE_ID
            );
            println!("virtio-blk BAR0: {VIRTIO_BLK_BAR0_GPA:#x}");
            println!("virtio-blk capacity sectors: {VIRTIO_BLK_CAPACITY_SECTORS}");
            println!(
                "virtio-blk driver features: {:#x}",
                result.driver_features()
            );
            println!("virtio-blk device status: {:#04x}", result.status());
            println!("virtio-blk queue enabled: {}", result.queue_enabled());
            println!(
                "virtio-blk completion: id={} len={} sector={}",
                result.completion().descriptor_id(),
                result.completion().length(),
                result.completion().sector()
            );
            println!(
                "virtio-blk used: idx={} id={} len={}",
                result.used_idx(),
                result.used_id(),
                result.used_len()
            );
            println!("virtio-blk request status: {}", result.request_status());
            println!(
                "virtio-blk data boundary: first={:?} last={:?}",
                &result.data()[..16],
                &result.data()[result.data().len() - 8..]
            );
            println!("virtio-blk proof: {:?}", result.proof());
            println!("virtio-blk port-I/O exits: {}", result.io_exits().len());
            println!("virtio-blk MMIO exits: {}", result.mmio_exits().len());
            println!("virtio-blk terminal RIP: {:#x}", result.terminal_rip());
            println!("{}", result.report());

            if result.driver_features() == VIRTIO_F_VERSION_1
                && result.queue_enabled()
                && result.completion().descriptor_id() == 0
                && result.completion().length() == 513
                && result.completion().sector() == 0
                && result.used_idx() == 1
                && result.used_id() == 0
                && result.used_len() == 513
                && result.request_status() == VIRTIO_BLK_S_OK
                && result.data() == deterministic_sector()
                && result.proof() == VIRTIO_BLK_PROOF
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: virtio-blk PCI read proof did not match expected state");
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
