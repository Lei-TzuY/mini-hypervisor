use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::two_vcpu_targeted_msi_fixture::{
    run_two_vcpu_targeted_msi_guest, FIRST_PROOF, SECOND_PROOF, TARGET_MSI_ADDRESS, TARGET_MSI_DATA,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{PortIoDirection, PortIoExit};

#[test]
fn targeted_msi_reaches_only_the_second_thread_owned_vcpu() {
    match run_two_vcpu_targeted_msi_guest() {
        Ok(result) => {
            assert_eq!(result.msi_address(), TARGET_MSI_ADDRESS);
            assert_eq!(result.msi_data(), TARGET_MSI_DATA);
            assert_eq!(result.msi_delivery_count(), 1);
            assert_eq!(result.second_mp_state(), 0);
            assert_eq!(result.first_proof(), FIRST_PROOF);
            assert_eq!(result.second_proof(), SECOND_PROOF);
            assert_debug_sequence(result.first_io_exits(), FIRST_PROOF);
            assert_debug_sequence(result.second_io_exits(), SECOND_PROOF);

            assert_eq!(result.first_barrier_rflags() & 0x2, 0x2);
            assert_eq!(
                result.first_barrier_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
            assert_eq!(result.second_ready_rflags() & 0x2, 0x2);
            assert_eq!(
                result.second_ready_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                0
            );
            assert_eq!(result.second_completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.second_completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping targeted MSI two-vCPU integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("targeted MSI two-vCPU execution failed unexpectedly: {error}"),
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
