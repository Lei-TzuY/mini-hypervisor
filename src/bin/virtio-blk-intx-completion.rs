use mini_hypervisor::config::VmConfig;
use mini_hypervisor::portio::pci::virtio_blk::VIRTIO_BLK_SECTOR_SIZE;
use mini_hypervisor::portio::virtio_blk_completion_interrupt_fixture::{
    run_virtio_blk_completion_interrupt_guest, VIRTIO_BLK_INTERRUPT_PROOF,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_virtio_blk_completion_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            println!("virtio-blk INTx GSI: {}", result.gsi());
            println!("virtio-blk INTx vector: {:#x}", result.vector());
            println!("virtio-blk INTx assert events: {}", result.assert_count());
            println!("virtio-blk INTx deassert events: {}", result.deassert_count());
            println!("virtio-blk INTx LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("virtio-blk INTx LAPIC LINT0: {:#x}", result.lapic_lint0());
            println!(
                "virtio-blk INTx completion: {}/{}/{}",
                result.completion().descriptor_id(),
                result.completion().length(),
                result.completion().sector()
            );
            println!(
                "virtio-blk INTx used: {}/{}/{}",
                result.used_idx(),
                result.used_id(),
                result.used_len()
            );
            println!("virtio-blk INTx request status: {}", result.request_status());
            println!("virtio-blk INTx data bytes: {}", result.data().len());
            println!("virtio-blk INTx port-I/O exits: {}", result.io_exits().len());
            println!("virtio-blk INTx MMIO exits: {}", result.mmio_exits().len());
            println!("virtio-blk INTx completion rflags: {:#x}", result.completion_rflags());
            println!("virtio-blk INTx proof: {:?}", result.proof());
            if result.proof() != VIRTIO_BLK_INTERRUPT_PROOF
                || result.data().len() != VIRTIO_BLK_SECTOR_SIZE
            {
                eprintln!("error: virtio-blk INTx result did not match fixed proof contract");
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
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
