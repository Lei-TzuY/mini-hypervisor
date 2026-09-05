from pathlib import Path

path = Path("src/kvm/ioeventfd_roundtrip.rs")
text = path.read_text()

old = '''    let bridge_worker = std::thread::spawn(move || -> io::Result<u64> {
        let count = wait_eventfd_value(&doorbell_reader, IOEVENTFD_WAIT_TIMEOUT_MILLIS)?;
        if count != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ioeventfd doorbell counter was {count}; expected exactly 1"),
            ));
        }
        irq_signal.signal()?;
        Ok(count)
    });
'''
new = '''    let (bridge_complete_tx, bridge_complete_rx) = std::sync::mpsc::channel::<()>();
    let bridge_worker = std::thread::spawn(move || -> io::Result<u64> {
        let count = wait_eventfd_value(&doorbell_reader, IOEVENTFD_WAIT_TIMEOUT_MILLIS)?;
        if count != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ioeventfd doorbell counter was {count}; expected exactly 1"),
            ));
        }
        irq_signal.signal()?;
        bridge_complete_tx.send(()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "ioeventfd bridge completion receiver disappeared before IRQFD signal acknowledgement",
            )
        })?;
        Ok(count)
    });
'''
assert text.count(old) == 1, f"bridge worker replacement count={text.count(old)}"
text = text.replace(old, new)

old = '''        let armed = vcpu.registers()?;
        require_interrupt_disabled_flags("ioeventfd round-trip armed state", armed.rflags)?;

        let handler_io = run_expected_debug_output(
'''
new = '''        let armed = vcpu.registers()?;
        require_interrupt_disabled_flags("ioeventfd round-trip armed state", armed.rflags)?;

        // A proves the KVM_IOEVENTFD MMIO write completed architecturally, but the userspace bridge
        // still has to consume the eventfd counter and signal the irqfd. Require that exact bridge
        // completion before re-entering the adjacent sti;hlt handoff so host scheduling cannot let
        // the mainline resume before the irqfd edge has become pending.
        wait_for_bridge_irqfd_signal(
            &bridge_complete_rx,
            std::time::Duration::from_millis(IOEVENTFD_WAIT_TIMEOUT_MILLIS as u64),
        )?;

        let handler_io = run_expected_debug_output(
'''
assert text.count(old) == 1, f"armed barrier replacement count={text.count(old)}"
text = text.replace(old, new)

marker = '''fn roundtrip_vm_error(operation: &'static str, source: io::Error) -> Error {
'''
helper = '''fn wait_for_bridge_irqfd_signal(
    receiver: &std::sync::mpsc::Receiver<()>,
    timeout: std::time::Duration,
) -> Result<(), Error> {
    match receiver.recv_timeout(timeout) {
        Ok(()) => Ok(()),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(verification_error(
            "ioeventfd/irqfd bridge completion",
            "timed out before the doorbell event was consumed and signaled through irqfd",
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(verification_error(
            "ioeventfd/irqfd bridge completion",
            "bridge worker disconnected before confirming the irqfd signal",
        )),
    }
}

'''
assert text.count(marker) == 1, f"helper insertion marker count={text.count(marker)}"
text = text.replace(marker, helper + marker)

test_marker = '''    #[test]
    fn deterministic_roundtrip_guest_places_doorbell_before_if_enable_handoff() {
'''
test = '''    #[test]
    fn bridge_completion_handshake_fails_closed_until_irqfd_signal_is_confirmed() {
        let (pending_tx, pending_rx) = std::sync::mpsc::channel::<()>();
        assert!(wait_for_bridge_irqfd_signal(&pending_rx, std::time::Duration::ZERO).is_err());
        pending_tx.send(()).unwrap();
        assert!(wait_for_bridge_irqfd_signal(&pending_rx, std::time::Duration::ZERO).is_ok());

        let (dropped_tx, dropped_rx) = std::sync::mpsc::channel::<()>();
        drop(dropped_tx);
        assert!(wait_for_bridge_irqfd_signal(&dropped_rx, std::time::Duration::ZERO).is_err());
    }

'''
assert text.count(test_marker) == 1, f"test insertion marker count={text.count(test_marker)}"
text = text.replace(test_marker, test + test_marker)

path.write_text(text)
