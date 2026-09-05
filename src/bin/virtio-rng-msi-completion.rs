use mini_hypervisor::config::VmConfig;
use mini_hypervisor::portio::pci::virtio::{VIRTIO_F_VERSION_1, VIRTIO_RNG_TEST_PAYLOAD};
use mini_hypervisor::portio::virtio_rng_msi_completion_fixture::{
    run_virtio_rng_msi_completion_guest, VIRTIO_RNG_MSI_ADDRESS, VIRTIO_RNG_MSI_DATA,
    VIRTIO_RNG_MSI_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_virtio_rng_msi_completion_guest(VmConfig::default()) {
        Ok(result) => {
            println!("virtio-rng MSI address: {:#x}", result.msi_address());
            println!("virtio-rng MSI data: {:#x}", result.msi_data());
            println!("virtio-rng MSI vector: {:#x}", result.vector());
            println!(
                "virtio-rng MSI delivery count: {}",
                result.msi_delivery_count()
            );
            println!("virtio-rng MSI LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!(
                "virtio-rng MSI used: idx={} id={} len={}",
                result.used_idx(),
                result.used_id(),
                result.used_len()
            );
            println!("virtio-rng MSI payload: {:?}", result.payload());
            println!("virtio-rng MSI proof: {:?}", result.proof());
            println!("virtio-rng MSI port-I/O exits: {}", result.io_exits().len());
            println!("virtio-rng MSI MMIO exits: {}", result.mmio_exits().len());
            println!(
                "virtio-rng MSI completion rflags: {:#x}",
                result.completion_rflags()
            );

            if result.driver_features() == VIRTIO_F_VERSION_1
                && result.queue_enabled()
                && result.payload() == VIRTIO_RNG_TEST_PAYLOAD
                && result.proof() == VIRTIO_RNG_MSI_PROOF
                && result.msi_address() == VIRTIO_RNG_MSI_ADDRESS
                && result.msi_data() == VIRTIO_RNG_MSI_DATA
                && result.msi_delivery_count() == 1
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: virtio-rng MSI completion proof did not match expected state");
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
