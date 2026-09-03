use mini_hypervisor::vcpu::VcpuExit;

const KVM_EXIT_UNKNOWN: u32 = 0;

#[test]
fn kvm_unknown_exit_is_typed_without_collapsing_other_unhandled_reasons() {
    assert_eq!(VcpuExit::from_raw(KVM_EXIT_UNKNOWN), VcpuExit::KvmUnknown);
    assert_eq!(VcpuExit::KvmUnknown.reason(), KVM_EXIT_UNKNOWN);

    let unsupported_reason = 0xfeed_beef;
    assert_eq!(
        VcpuExit::from_raw(unsupported_reason),
        VcpuExit::Unhandled {
            reason: unsupported_reason,
        }
    );
}
