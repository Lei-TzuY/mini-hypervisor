use mini_hypervisor::error::VmExitError;

#[test]
fn unhandled_exit_error_owns_ordered_completed_exit_trace() {
    let error = VmExitError::Unhandled {
        vcpu_id: 7,
        reason: 0xfeed_beef,
        rip: 0x1234,
        rflags: 0x2,
        exit_reasons: vec![2, 0xfeed_beef],
    };

    assert!(matches!(
        error,
        VmExitError::Unhandled {
            vcpu_id: 7,
            reason: 0xfeed_beef,
            rip: 0x1234,
            rflags: 0x2,
            exit_reasons,
        } if exit_reasons == [2, 0xfeed_beef]
    ));
}
