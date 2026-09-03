use mini_hypervisor::vcpu::VcpuExit;

const KVM_EXIT_INTERNAL_ERROR: u32 = 17;

#[test]
fn internal_error_exit_is_typed_and_round_trips_raw_reason() {
    assert_eq!(
        VcpuExit::from_raw(KVM_EXIT_INTERNAL_ERROR),
        VcpuExit::InternalError
    );
    assert_eq!(VcpuExit::InternalError.reason(), KVM_EXIT_INTERNAL_ERROR);
}
