use mini_hypervisor::vcpu::VcpuExit;

const KVM_EXIT_EXCEPTION: u32 = 1;

#[test]
fn exception_exit_reason_round_trips_through_typed_public_api() {
    let exit = VcpuExit::from_raw(KVM_EXIT_EXCEPTION);

    assert_eq!(exit, VcpuExit::Exception);
    assert_eq!(exit.reason(), KVM_EXIT_EXCEPTION);
}
