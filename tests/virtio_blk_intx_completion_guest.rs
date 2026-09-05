use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::pci::virtio::VIRTIO_F_VERSION_1;
use mini_hypervisor::portio::pci::virtio_blk::{
    deterministic_sector, VIRTIO_BLK_SECTOR_SIZE, VIRTIO_BLK_S_OK,
};
use mini_hypervisor::portio::virtio_blk_completion_interrupt_fixture::{
    run_virtio_blk_completion_interrupt_guest, VIRTIO_BLK_INTERRUPT_PROOF,
};

const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

#[test]
fn virtio_blk_read_completes_through_one_intx_lifecycle() {
    match run_virtio_blk_completion_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.gsi(), 0);
            assert_eq!(result.vector(), 0x40);
            assert_eq!(result.assert_count(), 1);
            assert_eq!(result.deassert_count(), 1);
            assert_eq!(result.completion().descriptor_id(), 0);
            assert_eq!(result.completion().length(), (VIRTIO_BLK_SECTOR_SIZE + 1) as u32);
            assert_eq!(result.completion().sector(), 0);
            assert_eq!(result.used_idx(), 1);
            assert_eq!(result.used_id(), 0);
            assert_eq!(result.used_len(), (VIRTIO_BLK_SECTOR_SIZE + 1) as u32);
            assert_eq!(result.request_status(), VIRTIO_BLK_S_OK);
            assert_eq!(result.driver_features(), VIRTIO_F_VERSION_1);
            assert!(result.queue_enabled());
            assert_eq!(result.data(), deterministic_sector());
            assert_eq!(result.proof(), VIRTIO_BLK_INTERRUPT_PROOF);
            assert_eq!(result.io_exits().len(), 21);
            assert_eq!(result.mmio_exits().len(), 22);
            assert_eq!(
                result.lapic_spiv() & APIC_SPIV_SOFTWARE_ENABLE,
                APIC_SPIV_SOFTWARE_ENABLE
            );
            assert_eq!(
                result.lapic_lint0() & APIC_LVT_DELIVERY_MODE_MASK,
                APIC_LVT_DELIVERY_MODE_EXTINT
            );
            assert_eq!(result.lapic_lint0() & APIC_LVT_MASKED, 0);
            assert_eq!(result.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping virtio-blk INTx integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("virtio-blk INTx guest execution failed unexpectedly: {error}"),
    }
}
