use crate::error::{Error, PortIoError};
use crate::vcpu::{PortIoDirection, PortIoExit};

pub const DEBUG_PORT: u16 = 0x00e9;

#[derive(Debug, Default)]
pub struct PortIoBus {
    debug_port: Option<DebugPort>,
}

impl PortIoBus {
    #[must_use]
    pub const fn empty() -> Self {
        Self { debug_port: None }
    }

    #[must_use]
    pub fn with_debug_port() -> Self {
        Self {
            debug_port: Some(DebugPort::default()),
        }
    }

    pub fn dispatch(&mut self, io: &PortIoExit) -> Result<(), Error> {
        match self.debug_port.as_mut() {
            Some(device) if io.port() == DEBUG_PORT => device.handle(io).map_err(Error::PortIo),
            _ => Err(Error::PortIo(PortIoError::UnhandledPort {
                port: io.port(),
                direction: io.direction().raw(),
                size: io.size(),
                count: io.count(),
            })),
        }
    }

    #[must_use]
    pub fn debug_output(&self) -> Option<&[u8]> {
        self.debug_port.as_ref().map(DebugPort::bytes)
    }
}

#[derive(Debug, Default)]
struct DebugPort {
    bytes: Vec<u8>,
}

impl DebugPort {
    fn handle(&mut self, io: &PortIoExit) -> Result<(), PortIoError> {
        if io.direction() != PortIoDirection::Out || io.size() != 1 || io.count() != 1 {
            return Err(PortIoError::UnsupportedDebugAccess {
                port: io.port(),
                direction: io.direction().raw(),
                size: io.size(),
                count: io.count(),
            });
        }

        if io.output_data().len() != 1 {
            return Err(PortIoError::InvalidOutputPayload {
                port: io.port(),
                expected: 1,
                actual: io.output_data().len(),
            });
        }

        self.bytes.push(io.output_data()[0]);
        Ok(())
    }

    fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(port: u16, size: u8, count: u32, bytes: &[u8]) -> PortIoExit {
        PortIoExit::new(PortIoDirection::Out, size, port, count, bytes.to_vec())
    }

    #[test]
    fn debug_port_captures_one_byte_output() {
        let mut bus = PortIoBus::with_debug_port();
        let io = output(DEBUG_PORT, 1, 1, b"K");

        bus.dispatch(&io).unwrap();

        assert_eq!(bus.debug_output(), Some(&b"K"[..]));
    }

    #[test]
    fn rejects_unknown_port_with_full_metadata() {
        let mut bus = PortIoBus::with_debug_port();
        let io = output(0x1234, 1, 1, b"X");

        assert!(matches!(
            bus.dispatch(&io),
            Err(Error::PortIo(PortIoError::UnhandledPort {
                port: 0x1234,
                direction: 1,
                size: 1,
                count: 1,
            }))
        ));
    }

    #[test]
    fn rejects_debug_port_input() {
        let mut bus = PortIoBus::with_debug_port();
        let io = PortIoExit::new(PortIoDirection::In, 1, DEBUG_PORT, 1, Vec::new());

        assert!(matches!(
            bus.dispatch(&io),
            Err(Error::PortIo(PortIoError::UnsupportedDebugAccess {
                port: DEBUG_PORT,
                direction: 0,
                size: 1,
                count: 1,
            }))
        ));
    }

    #[test]
    fn rejects_debug_port_wide_output() {
        let mut bus = PortIoBus::with_debug_port();
        let io = output(DEBUG_PORT, 2, 1, &[0x34, 0x12]);

        assert!(matches!(
            bus.dispatch(&io),
            Err(Error::PortIo(PortIoError::UnsupportedDebugAccess {
                port: DEBUG_PORT,
                direction: 1,
                size: 2,
                count: 1,
            }))
        ));
    }

    #[test]
    fn rejects_debug_port_multi_count_output() {
        let mut bus = PortIoBus::with_debug_port();
        let io = output(DEBUG_PORT, 1, 2, b"AB");

        assert!(matches!(
            bus.dispatch(&io),
            Err(Error::PortIo(PortIoError::UnsupportedDebugAccess {
                port: DEBUG_PORT,
                direction: 1,
                size: 1,
                count: 2,
            }))
        ));
    }

    #[test]
    fn rejects_mismatched_output_payload_length() {
        let mut bus = PortIoBus::with_debug_port();
        let io = output(DEBUG_PORT, 1, 1, b"AB");

        assert!(matches!(
            bus.dispatch(&io),
            Err(Error::PortIo(PortIoError::InvalidOutputPayload {
                port: DEBUG_PORT,
                expected: 1,
                actual: 2,
            }))
        ));
    }
}
