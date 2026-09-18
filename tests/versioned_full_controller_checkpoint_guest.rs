use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::state_snapshot::{
    run_versioned_full_controller_checkpoint_guest, CONTROLLER_CHECKPOINT_CAPTURE_RIP,
    CONTROLLER_CHECKPOINT_PAGE, FULL_CONTROLLER_CHECKPOINT_PROOF, FULL_CONTROLLER_IOAPIC_VECTOR,
    VERSIONED_FULL_CONTROLLER_CHECKPOINT_VERSION, VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT,
};
use mini_hypervisor::vcpu::PortIoDirection;

const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

#[test]
fn versioned_full_controller_checkpoint_decodes_materializes_restores_and_resumes() {
    match run_versioned_full_controller_checkpoint_guest() {
        Ok(result) => {
            assert_eq!(
                result.schema_version(),
                VERSIONED_FULL_CONTROLLER_CHECKPOINT_VERSION
            );
            assert!(result.encoded_len() > 4096);
            assert_eq!(result.page_count(), 1);
            assert!(result.msr_count() <= VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT);
            assert!(result.canonical_roundtrip());

            let checkpoint = result.checkpoint();
            assert_eq!(checkpoint.capture().rip(), CONTROLLER_CHECKPOINT_CAPTURE_RIP);
            assert_eq!(checkpoint.capture().rflags() & 0x2, 0x2);
            assert_eq!(
                checkpoint.capture().rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                0
            );

            assert_eq!(
                checkpoint.corruption().page_exact(CONTROLLER_CHECKPOINT_PAGE),
                Some(false)
            );
            assert!(!checkpoint.corruption().vcpu_exact());
            assert!(!checkpoint.corruption().master_pic_exact());
            assert!(!checkpoint.corruption().slave_pic_exact());
            assert!(!checkpoint.corruption().ioapic_exact());
            assert!(!checkpoint.corruption().lapic_exact());

            assert_eq!(
                checkpoint.restored().page_exact(CONTROLLER_CHECKPOINT_PAGE),
                Some(true)
            );
            assert!(checkpoint.restored().is_exact_match());

            assert_eq!(checkpoint.captured_master_pic_imr(), 0xfb);
            assert_eq!(checkpoint.captured_slave_pic_imr(), 0xfe);
            assert_eq!(
                checkpoint.captured_ioapic_pin16(),
                u64::from(FULL_CONTROLLER_IOAPIC_VECTOR)
            );
            assert_eq!(
                checkpoint.captured_lapic_spiv() & APIC_SPIV_SOFTWARE_ENABLE,
                APIC_SPIV_SOFTWARE_ENABLE
            );
            assert_eq!(
                checkpoint.captured_lapic_lint0() & APIC_LVT_DELIVERY_MODE_MASK,
                APIC_LVT_DELIVERY_MODE_EXTINT
            );
            assert_eq!(checkpoint.captured_lapic_lint0() & APIC_LVT_MASKED, 0);

            for rflags in [
                checkpoint.slave_armed_rflags(),
                checkpoint.ioapic_armed_rflags(),
                checkpoint.completion_rflags(),
            ] {
                assert_eq!(rflags & 0x2, 0x2);
                assert_eq!(
                    rflags & X86_RFLAGS_INTERRUPT_ENABLE,
                    X86_RFLAGS_INTERRUPT_ENABLE
                );
            }

            assert_eq!(checkpoint.proof(), FULL_CONTROLLER_CHECKPOINT_PROOF);
            assert_eq!(
                checkpoint.io_exits().len(),
                FULL_CONTROLLER_CHECKPOINT_PROOF.len()
            );
            for (io, expected) in checkpoint
                .io_exits()
                .iter()
                .zip(FULL_CONTROLLER_CHECKPOINT_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping versioned full-controller checkpoint integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!(
            "versioned full-controller checkpoint guest execution failed unexpectedly: {error}"
        ),
    }
}
