use mini_hypervisor::vcpu::VcpuExit;

const KVM_EXIT_MMIO: u32 = 6;

#[test]
fn mmio_exit_is_typed_without_collapsing_other_unhandled_reasons() {
    assert_eq!(VcpuExit::from_raw(KVM_EXIT_MMIO), VcpuExit::Mmio);
    assert_eq!(VcpuExit::Mmio.reason(), KVM_EXIT_MMIO);

    let unsupported_reason = 0xfeed_beef;
    assert_eq!(
        VcpuExit::from_raw(unsupported_reason),
        VcpuExit::Unhandled {
            reason: unsupported_reason,
        }
    );
}
