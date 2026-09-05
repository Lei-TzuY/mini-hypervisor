use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::two_vcpu_guest_ipi_fixture::{
    run_two_vcpu_guest_ipi, FIRST_PROOF, ICR_HIGH_VALUE, ICR_LOW_VALUE, LAPIC_GPA,
    LAPIC_VIRTUAL_PAGE, SECOND_PROOF, TARGET_APIC_ID, TARGET_VECTOR,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{PortIoDirection, PortIoExit};

#[test]
fn guest_xapic_icr_targets_only_the_thread_owned_second_vcpu() {
    match run_two_vcpu_guest_ipi() {
        Ok(result) => {
            assert_eq!(LAPIC_VIRTUAL_PAGE, 0x50_0000);
            assert_eq!(LAPIC_GPA, 0xfee0_0000);
            assert_eq!(TARGET_APIC_ID, 1);
            assert_eq!(TARGET_VECTOR, 0x52);
            assert_eq!(ICR_HIGH_VALUE, 0x0100_0000);
            assert_eq!(ICR_LOW_VALUE, 0x52);
            assert_eq!(result.second_mp_state(), 0);
            assert_eq!(result.first_proof(), FIRST_PROOF);
            assert_eq!(result.second_proof(), SECOND_PROOF);
            assert_debug_sequence(result.first_io_exits(), FIRST_PROOF);
            assert_debug_sequence(result.second_io_exits(), SECOND_PROOF);

            for rflags in [
                result.first_barrier_rflags(),
                result.first_send_rflags(),
                result.first_completion_rflags(),
                result.second_completion_rflags(),
            ] {
                assert_eq!(rflags & 0x2, 0x2);
                assert_eq!(
                    rflags & X86_RFLAGS_INTERRUPT_ENABLE,
                    X86_RFLAGS_INTERRUPT_ENABLE
                );
            }
            assert_eq!(result.second_ready_rflags() & 0x2, 0x2);
            assert_eq!(
                result.second_ready_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                0
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping guest IPI two-vCPU integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("guest IPI two-vCPU execution failed unexpectedly: {error}"),
    }
}

fn assert_debug_sequence(exits: &[PortIoExit], expected: &[u8]) {
    assert_eq!(exits.len(), expected.len());
    for (exit, expected_byte) in exits.iter().zip(expected.iter().copied()) {
        assert_eq!(exit.direction(), PortIoDirection::Out);
        assert_eq!(exit.port(), DEBUG_PORT);
        assert_eq!(exit.size(), 1);
        assert_eq!(exit.count(), 1);
        assert_eq!(exit.output_data(), &[expected_byte]);
    }
}
