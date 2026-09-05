use mini_hypervisor::config::VmConfig;
use mini_hypervisor::portio::pci::virtio::{VIRTIO_F_VERSION_1, VIRTIO_RNG_TEST_PAYLOAD};
use mini_hypervisor::portio::virtio_rng_completion_interrupt_fixture::{
    run_virtio_rng_completion_interrupt_guest, VIRTIO_RNG_INTERRUPT_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_virtio_rng_completion_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            println!("virtio-rng interrupt GSI: {}", result.gsi());
            println!("virtio-rng interrupt vector: {:#x}", result.vector());
            println!(
                "virtio-rng interrupt lifecycle: assert={} deassert={}",
                result.assert_count(),
                result.deassert_count()
            );
            println!(
                "virtio-rng interrupt LAPIC SPIV: {:#x}",
                result.lapic_spiv()
            );
            println!(
                "virtio-rng interrupt LAPIC LINT0: {:#x}",
                result.lapic_lint0()
            );
            println!(
                "virtio-rng interrupt used: idx={} id={} len={}",
                result.used_idx(),
                result.used_id(),
                result.used_len()
            );
            println!("virtio-rng interrupt payload: {:?}", result.payload());
            println!("virtio-rng interrupt proof: {:?}", result.proof());
            println!(
                "virtio-rng interrupt port-I/O exits: {}",
                result.io_exits().len()
            );
            println!(
                "virtio-rng interrupt MMIO exits: {}",
                result.mmio_exits().len()
            );
            println!(
                "virtio-rng interrupt completion rflags: {:#x}",
                result.completion_rflags()
            );

            if result.driver_features() == VIRTIO_F_VERSION_1
                && result.queue_enabled()
                && result.payload() == VIRTIO_RNG_TEST_PAYLOAD
                && result.proof() == VIRTIO_RNG_INTERRUPT_PROOF
                && result.assert_count() == 1
                && result.deassert_count() == 1
            {
                ExitCode::SUCCESS
            } else {
                eprintln!(
                    "error: virtio-rng completion interrupt proof did not match expected state"
                );
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
