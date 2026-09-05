use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::PortIoDirection;

const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

#[test]
fn host_timer_wakes_sti_hlt_guest_through_pic_vector() {
    match KvmBackend::run_async_timer_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.gsi(), KvmBackend::ASYNC_TIMER_GSI);
            assert_eq!(result.vector(), KvmBackend::ASYNC_TIMER_VECTOR);
            assert_eq!(
                result.lapic_spiv() & APIC_SPIV_SOFTWARE_ENABLE,
                APIC_SPIV_SOFTWARE_ENABLE
            );
            assert_eq!(
                result.lapic_lint0() & APIC_LVT_DELIVERY_MODE_MASK,
                APIC_LVT_DELIVERY_MODE_EXTINT
            );
            assert_eq!(result.lapic_lint0() & APIC_LVT_MASKED, 0);

            assert_eq!(result.armed_rflags() & 0x2, 0x2);
            assert_eq!(
                result.armed_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                0,
                "A barrier must still be under CLI before the sti;hlt handoff"
            );
            assert_eq!(result.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );

            assert_eq!(result.proof(), KvmBackend::ASYNC_TIMER_PROOF);
            assert_eq!(result.io_exits().len(), KvmBackend::ASYNC_TIMER_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(KvmBackend::ASYNC_TIMER_PROOF.iter().copied())
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
                "skipping async timer interrupt integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("async timer interrupt guest execution failed unexpectedly: {error}"),
    }
}
