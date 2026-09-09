use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::state_snapshot::{
    run_controller_checkpoint_guest, CONTROLLER_CHECKPOINT_CAPTURE_RIP,
    CONTROLLER_CHECKPOINT_PAGE, CONTROLLER_CHECKPOINT_PROOF,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

#[test]
fn controller_checkpoint_restores_page_vcpu_pic_lapic_and_resumes_interrupt_delivery() {
    match run_controller_checkpoint_guest() {
        Ok(result) => {
            assert_eq!(result.capture().exit(), VcpuExit::Hlt);
            assert_eq!(result.capture().rip(), CONTROLLER_CHECKPOINT_CAPTURE_RIP);
            assert_eq!(result.capture().rflags() & 0x2, 0x2);
            assert_eq!(
                result.capture().rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                0
            );

            assert_eq!(
                result.corruption().page_exact(CONTROLLER_CHECKPOINT_PAGE),
                Some(false)
            );
            assert!(!result.corruption().vcpu_exact());
            assert!(!result.corruption().master_pic_exact());
            assert!(!result.corruption().lapic_exact());

            assert_eq!(
                result.restored().page_exact(CONTROLLER_CHECKPOINT_PAGE),
                Some(true)
            );
            assert!(result.restored().vcpu_exact());
            assert!(result.restored().master_pic_exact());
            assert!(result.restored().lapic_exact());
            assert!(result.restored().is_exact_match());

            assert_eq!(result.captured_pic_imr(), 0xfe);
            assert_eq!(
                result.captured_lapic_spiv() & APIC_SPIV_SOFTWARE_ENABLE,
                APIC_SPIV_SOFTWARE_ENABLE
            );
            assert_eq!(
                result.captured_lapic_lint0() & APIC_LVT_DELIVERY_MODE_MASK,
                APIC_LVT_DELIVERY_MODE_EXTINT
            );
            assert_eq!(result.captured_lapic_lint0() & APIC_LVT_MASKED, 0);
            for rflags in [result.armed_rflags(), result.completion_rflags()] {
                assert_eq!(rflags & 0x2, 0x2);
                assert_eq!(
                    rflags & X86_RFLAGS_INTERRUPT_ENABLE,
                    X86_RFLAGS_INTERRUPT_ENABLE
                );
            }

            assert_eq!(result.proof(), CONTROLLER_CHECKPOINT_PROOF);
            assert_eq!(result.io_exits().len(), CONTROLLER_CHECKPOINT_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(CONTROLLER_CHECKPOINT_PROOF.iter().copied())
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
                "skipping controller checkpoint integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("controller checkpoint guest execution failed unexpectedly: {error}"),
    }
}
