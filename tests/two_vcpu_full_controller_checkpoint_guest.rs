use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::state_snapshot::{
    run_two_vcpu_full_controller_checkpoint_guest, TWO_VCPU_CHECKPOINT_FIRST_ID,
    TWO_VCPU_CHECKPOINT_FIRST_PROOF, TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE,
    TWO_VCPU_CHECKPOINT_OWNERSHIP_SET, TWO_VCPU_CHECKPOINT_SECOND_ID,
    TWO_VCPU_CHECKPOINT_SECOND_PROOF, TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
    TWO_VCPU_CHECKPOINT_SHARED_PAGE,
};

#[test]
fn two_vcpu_full_controller_checkpoint_restores_both_vcpus_both_lapics_and_vm_irqchip() {
    match run_two_vcpu_full_controller_checkpoint_guest() {
        Ok(result) => {
            assert_eq!(result.captured_pages(), TWO_VCPU_CHECKPOINT_OWNERSHIP_SET);

            assert_eq!(result.first_capture_rip(), 0x1000f);
            assert_eq!(result.second_capture_rip(), 0x11007);
            assert_eq!(result.first_capture_rflags() & 0x2, 0x2);
            assert_eq!(result.second_capture_rflags() & 0x2, 0x2);

            for page in [
                TWO_VCPU_CHECKPOINT_SHARED_PAGE,
                TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE,
                TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
            ] {
                assert_eq!(result.corruption().page_exact(page), Some(false));
                assert_eq!(result.restored().page_exact(page), Some(true));
            }

            for id in [TWO_VCPU_CHECKPOINT_FIRST_ID, TWO_VCPU_CHECKPOINT_SECOND_ID] {
                assert_eq!(result.corruption().vcpu_exact(id), Some(false));
                assert_eq!(result.corruption().mp_state_exact(id), Some(false));
                assert_eq!(result.corruption().lapic_exact(id), Some(false));
                assert_eq!(result.restored().vcpu_exact(id), Some(true));
                assert_eq!(result.restored().mp_state_exact(id), Some(true));
                assert_eq!(result.restored().lapic_exact(id), Some(true));
            }

            assert!(!result.corruption().master_pic_exact());
            assert!(!result.corruption().slave_pic_exact());
            assert!(!result.corruption().ioapic_exact());
            assert!(result.restored().master_pic_exact());
            assert!(result.restored().slave_pic_exact());
            assert!(result.restored().ioapic_exact());
            assert!(result.restored().is_exact_match());

            assert_eq!(result.first_proof(), TWO_VCPU_CHECKPOINT_FIRST_PROOF);
            assert_eq!(result.second_proof(), TWO_VCPU_CHECKPOINT_SECOND_PROOF);
            assert_eq!(result.first_completion_rip(), 0x10024);
            assert_eq!(result.second_completion_rip(), 0x1101c);
            assert_eq!(result.first_completion_rflags() & 0x2, 0x2);
            assert_eq!(result.second_completion_rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping two-vCPU full-controller checkpoint assertion: /dev/kvm unavailable"
            );
        }
        Err(error) => {
            panic!("two-vCPU full-controller checkpoint failed unexpectedly: {error}")
        }
    }
}
