use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::state_snapshot::{
    run_two_vcpu_two_device_transaction_guest, TWO_VCPU_CHECKPOINT_FIRST_ID,
    TWO_VCPU_CHECKPOINT_FIRST_PROOF, TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE,
    TWO_VCPU_CHECKPOINT_OWNERSHIP_SET, TWO_VCPU_CHECKPOINT_SECOND_ID,
    TWO_VCPU_CHECKPOINT_SECOND_PROOF, TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
    TWO_VCPU_CHECKPOINT_SHARED_PAGE,
};

const FIRST_BAR: u64 = 0x1000_0000;
const SECOND_BAR: u64 = 0x1000_1000;

#[test]
fn two_vcpu_two_device_transaction_restores_every_owned_layer_and_reconstructs_quiescent_acceleration(
) {
    match run_two_vcpu_two_device_transaction_guest() {
        Ok(result) => {
            assert_eq!(result.bars(), [FIRST_BAR, SECOND_BAR]);
            assert_eq!(result.captured_statuses(), [0x01, 0x03]);
            assert_eq!(result.restored_statuses(), [0x01, 0x03]);
            assert_eq!(result.capture_pending(), [false, false]);
            assert_eq!(result.reconstructed_pending(), [false, false]);

            let mutation = result.mutation();
            let restored = result.restored();
            for page in TWO_VCPU_CHECKPOINT_OWNERSHIP_SET {
                assert_eq!(mutation.controller().page_exact(page), Some(false));
                assert_eq!(restored.controller().page_exact(page), Some(true));
            }
            assert_eq!(
                mutation
                    .controller()
                    .page_exact(TWO_VCPU_CHECKPOINT_SHARED_PAGE),
                Some(false)
            );
            assert_eq!(
                mutation
                    .controller()
                    .page_exact(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE),
                Some(false)
            );
            assert_eq!(
                mutation
                    .controller()
                    .page_exact(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE),
                Some(false)
            );

            for id in [TWO_VCPU_CHECKPOINT_FIRST_ID, TWO_VCPU_CHECKPOINT_SECOND_ID] {
                assert_eq!(mutation.controller().vcpu_exact(id), Some(false));
                assert_eq!(mutation.controller().mp_state_exact(id), Some(false));
                assert_eq!(mutation.controller().lapic_exact(id), Some(false));
                assert_eq!(restored.controller().vcpu_exact(id), Some(true));
                assert_eq!(restored.controller().mp_state_exact(id), Some(true));
                assert_eq!(restored.controller().lapic_exact(id), Some(true));
            }

            assert!(!mutation.controller().master_pic_exact());
            assert!(!mutation.controller().slave_pic_exact());
            assert!(!mutation.controller().ioapic_exact());
            assert_eq!(mutation.device_exact(FIRST_BAR), Some(false));
            assert_eq!(mutation.device_exact(SECOND_BAR), Some(false));
            assert!(!mutation.is_exact_match());

            assert!(restored.controller().master_pic_exact());
            assert!(restored.controller().slave_pic_exact());
            assert!(restored.controller().ioapic_exact());
            assert_eq!(restored.device_exact(FIRST_BAR), Some(true));
            assert_eq!(restored.device_exact(SECOND_BAR), Some(true));
            assert!(restored.is_exact_match());

            assert_eq!(result.first_capture_rip(), 0x1000e);
            assert_eq!(result.second_capture_rip(), 0x11006);
            assert_eq!(result.first_completion_rip(), 0x10023);
            assert_eq!(result.second_completion_rip(), 0x1101b);
            assert_eq!(result.first_proof(), TWO_VCPU_CHECKPOINT_FIRST_PROOF);
            assert_eq!(result.second_proof(), TWO_VCPU_CHECKPOINT_SECOND_PROOF);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping two-vCPU two-device transaction assertion: /dev/kvm unavailable");
        }
        Err(error) => panic!("two-vCPU two-device transaction failed unexpectedly: {error}"),
    }
}
