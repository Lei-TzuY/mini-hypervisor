use mini_hypervisor::vcpu::VcpuExit;

const KVM_EXIT_EXCEPTION: u32 = 1;

#[test]
fn kvm_exception_exit_is_typed_without_collapsing_other_unhandled_reasons() {
    assert_eq!(
        VcpuExit::from_raw(KVM_EXIT_EXCEPTION),
        VcpuExit::Exception
    );
    assert_eq!(VcpuExit::Exception.reason(), KVM_EXIT_EXCEPTION);

    let unsupported_reason = 0xfeed_beef;
    assert_eq!(
        VcpuExit::from_raw(unsupported_reason),
        VcpuExit::Unhandled {
            reason: unsupported_reason,
        }
    );
}
