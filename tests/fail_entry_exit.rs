use mini_hypervisor::vcpu::VcpuExit;

const KVM_EXIT_FAIL_ENTRY: u32 = 9;

#[test]
fn fail_entry_exit_is_typed_and_round_trips_raw_reason() {
    assert_eq!(VcpuExit::from_raw(KVM_EXIT_FAIL_ENTRY), VcpuExit::FailEntry);
    assert_eq!(VcpuExit::FailEntry.reason(), KVM_EXIT_FAIL_ENTRY);
}
