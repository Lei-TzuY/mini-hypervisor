use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::state_snapshot::{
    run_transaction_coupled_dual_device_write_readback_guest,
    TRANSACTION_COUPLED_WRITE_READBACK_PROOF,
};

#[test]
fn decoded_transaction_preserves_independent_dual_device_write_readback_on_the_restored_vm() {
    match run_transaction_coupled_dual_device_write_readback_guest() {
        Ok(result) => {
            assert_eq!(result.transaction_version(), 1);
            assert_eq!(result.bars(), [0x1000_0000, 0x1000_1000]);
            assert!(!result.mutation().is_exact_match());
            assert_eq!(
                result.mutation().device_exact(result.bars()[0]),
                Some(false)
            );
            assert_eq!(
                result.mutation().device_exact(result.bars()[1]),
                Some(false)
            );
            assert!(result.restored().is_exact_match());
            assert_eq!(result.queue_indices(), [[2, 2], [2, 2]]);
            assert_eq!(result.doorbell_events(), [2, 2]);
            assert_eq!(result.irqfd_signals(), [2, 2]);
            assert_eq!(result.readback()[0], result.write_payloads()[0]);
            assert_eq!(result.readback()[1], result.write_payloads()[1]);
            assert_eq!(result.backing()[0], result.write_payloads()[0]);
            assert_eq!(result.backing()[1], result.write_payloads()[1]);
            assert_ne!(result.backing()[0], result.backing()[1]);
            assert_eq!(result.proof(), TRANSACTION_COUPLED_WRITE_READBACK_PROOF);
            assert_eq!(result.completion_rflags() & 0x2, 0x2);
            assert_eq!(
                result.completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                X86_RFLAGS_INTERRUPT_ENABLE
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping coupled dual-device write/readback assertion: /dev/kvm unavailable"
            );
        }
        Err(error) => panic!("coupled dual-device write/readback failed unexpectedly: {error}"),
    }
}
