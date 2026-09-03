use mini_hypervisor::vcpu::VcpuInternalErrorSuberror;

#[test]
fn known_internal_error_suberrors_round_trip_exactly() {
    let cases = [
        (1, VcpuInternalErrorSuberror::Emulation),
        (2, VcpuInternalErrorSuberror::SimultaneousExceptions),
        (3, VcpuInternalErrorSuberror::DeliveryEvent),
        (4, VcpuInternalErrorSuberror::UnexpectedExitReason),
    ];

    for (raw, expected) in cases {
        let classified = VcpuInternalErrorSuberror::from_raw(raw);
        assert_eq!(classified, expected);
        assert_eq!(classified.raw(), raw);
    }
}

#[test]
fn unknown_internal_error_suberror_preserves_raw_value() {
    let raw = 0xfeed_beef;
    let classified = VcpuInternalErrorSuberror::from_raw(raw);

    assert_eq!(classified, VcpuInternalErrorSuberror::Unknown(raw));
    assert_eq!(classified.raw(), raw);
}
