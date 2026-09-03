use mini_hypervisor::vcpu::{VcpuExit, VcpuSystemEventType};

#[test]
fn system_event_exit_and_event_types_preserve_kvm_abi_values() {
    assert_eq!(VcpuExit::from_raw(24), VcpuExit::SystemEvent);
    assert_eq!(VcpuExit::SystemEvent.reason(), 24);

    let known = [
        (1, VcpuSystemEventType::Shutdown),
        (2, VcpuSystemEventType::Reset),
        (3, VcpuSystemEventType::Crash),
        (4, VcpuSystemEventType::Wakeup),
        (5, VcpuSystemEventType::Suspend),
        (6, VcpuSystemEventType::SevTerm),
        (7, VcpuSystemEventType::TdxFatal),
    ];
    for (raw, event_type) in known {
        assert_eq!(VcpuSystemEventType::from_raw(raw), event_type);
        assert_eq!(event_type.raw(), raw);
    }

    let unknown = VcpuSystemEventType::from_raw(0xfeed_beef);
    assert_eq!(unknown, VcpuSystemEventType::Unknown { raw: 0xfeed_beef });
    assert_eq!(unknown.raw(), 0xfeed_beef);
}
