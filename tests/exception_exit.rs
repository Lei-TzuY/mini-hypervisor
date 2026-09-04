use mini_hypervisor::vcpu::VcpuExit;

#[test]
fn kvm_exception_exit_round_trips_reason_one() {
    let exit = VcpuExit::from_raw(1);

    assert_eq!(exit, VcpuExit::Exception);
    assert_eq!(exit.reason(), 1);
}
