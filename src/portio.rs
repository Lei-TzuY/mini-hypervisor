#[path = "pci.rs"]
pub mod pci;
pub mod pci_fixture;
pub mod virtio_rng_completion_interrupt_fixture;
pub mod virtio_rng_fixture;

use crate::error::{Error, PortIoError};
use crate::vcpu::{PortIoDirection, PortIoExit};
use pci::{PciConfigMechanism1, PciConfigService};

pub const DEBUG_PORT: u16 = 0x00e9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortIoService {
    Output,
    Input(Vec<u8>),
}

#[derive(Debug, Default)]
pub struct PortIoBus {
    debug_port: Option<DebugPort>,
    pci_config: Option<PciConfigMechanism1>,
}

impl PortIoBus {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            debug_port: None,
            pci_config: None,
        }
    }

    #[must_use]
    pub fn with_debug_port() -> Self {
        Self {
            debug_port: Some(DebugPort::default()),
            pci_config: None,
        }
    }

    #[must_use]
    pub fn with_debug_port_input(input_byte: u8) -> Self {
        Self {
            debug_port: Some(DebugPort {
                input_byte,
                ..DebugPort::default()
            }),
            pci_config: None,
        }
    }

    #[must_use]
    pub fn with_debug_port_and_pci_config(pci_config: PciConfigMechanism1) -> Self {
        Self {
            debug_port: Some(DebugPort::default()),
            pci_config: Some(pci_config),
        }
    }

    pub fn dispatch(&mut self, io: &PortIoExit) -> Result<PortIoService, Error> {
        if io.port() == DEBUG_PORT {
            return match self.debug_port.as_mut() {
                Some(device) => device.handle(io).map_err(Error::PortIo),
                None => Err(Error::PortIo(unhandled(io))),
            };
        }

        if PciConfigMechanism1::handles_port(io.port()) {
            return match self.pci_config.as_mut() {
                Some(config) => config
                    .dispatch(io)
                    .map(convert_pci_service)
                    .map_err(Error::PortIo),
                None => Err(Error::PortIo(unhandled(io))),
            };
        }

        Err(Error::PortIo(unhandled(io)))
    }

    #[must_use]
    pub fn debug_output(&self) -> Option<&[u8]> {
        self.debug_port.as_ref().map(DebugPort::bytes)
    }
}

fn convert_pci_service(service: PciConfigService) -> PortIoService {
    match service {
        PciConfigService::Output => PortIoService::Output,
        PciConfigService::Input(bytes) => PortIoService::Input(bytes.to_vec()),
    }
}

fn unhandled(io: &PortIoExit) -> PortIoError {
    PortIoError::UnhandledPort {
        port: io.port(),
        direction: io.direction().raw(),
        size: io.size(),
        count: io.count(),
    }
}

#[derive(Debug, Default)]
struct DebugPort {
    bytes: Vec<u8>,
    input_byte: u8,
}

impl DebugPort {
    fn handle(&mut self, io: &PortIoExit) -> Result<PortIoService, PortIoError> {
        if io.size() != 1 || io.count() != 1 {
            return Err(PortIoError::UnsupportedDebugAccess {
                port: io.port(),
                direction: io.direction().raw(),
                size: io.size(),
                count: io.count(),
            });
        }

        match io.direction() {
            PortIoDirection::Out => {
                if io.output_data().len() != 1 {
                    return Err(PortIoError::InvalidOutputPayload {
                        port: io.port(),
                        expected: 1,
                        actual: io.output_data().len(),
                    });
                }

                self.bytes.push(io.output_data()[0]);
                Ok(PortIoService::Output)
            }
            PortIoDirection::In => Ok(PortIoService::Input(vec![self.input_byte])),
        }
    }

    fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use self::pci::{SyntheticPciFunction, PCI_CONFIG_ADDRESS_PORT, PCI_CONFIG_DATA_PORT};
    use super::*;

    fn output(port: u16, size: u8, count: u32, bytes: &[u8]) -> PortIoExit {
        PortIoExit::new(PortIoDirection::Out, size, port, count, bytes.to_vec())
    }

    fn input(port: u16, size: u8, count: u32) -> PortIoExit {
        PortIoExit::new(PortIoDirection::In, size, port, count, Vec::new())
    }

    #[test]
    fn debug_port_captures_one_byte_output() {
        let mut bus = PortIoBus::with_debug_port();
        let io = output(DEBUG_PORT, 1, 1, b"K");

        assert_eq!(bus.dispatch(&io).unwrap(), PortIoService::Output);
        assert_eq!(bus.debug_output(), Some(&b"K"[..]));
    }

    #[test]
    fn debug_port_returns_configured_one_byte_input() {
        let mut bus = PortIoBus::with_debug_port_input(b'R');
        let io = input(DEBUG_PORT, 1, 1);

        assert_eq!(bus.dispatch(&io).unwrap(), PortIoService::Input(vec![b'R']));
        assert_eq!(bus.debug_output(), Some(&[][..]));
    }

    #[test]
    fn pci_config_coexists_with_debug_port() {
        let mut bus = PortIoBus::with_debug_port_and_pci_config(PciConfigMechanism1::new(
            SyntheticPciFunction::new(0x1000_0000),
        ));
        let selector = pci::config_selector(0x00).to_le_bytes();

        assert_eq!(
            bus.dispatch(&output(PCI_CONFIG_ADDRESS_PORT, 4, 1, &selector))
                .unwrap(),
            PortIoService::Output
        );
        assert_eq!(
            bus.dispatch(&input(PCI_CONFIG_DATA_PORT, 4, 1)).unwrap(),
            PortIoService::Input(
                ((u32::from(pci::SYNTHETIC_PCI_DEVICE_ID) << 16)
                    | u32::from(pci::SYNTHETIC_PCI_VENDOR_ID))
                .to_le_bytes()
                .to_vec()
            )
        );

        assert_eq!(
            bus.dispatch(&output(DEBUG_PORT, 1, 1, b"P")).unwrap(),
            PortIoService::Output
        );
        assert_eq!(bus.debug_output(), Some(&b"P"[..]));
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
    fn rejects_debug_port_wide_input() {
        let mut bus = PortIoBus::with_debug_port_input(b'R');
        let io = input(DEBUG_PORT, 2, 1);

        assert!(matches!(
            bus.dispatch(&io),
            Err(Error::PortIo(PortIoError::UnsupportedDebugAccess {
                port: DEBUG_PORT,
                direction: 0,
                size: 2,
                count: 1,
            }))
        ));
    }

    #[test]
    fn rejects_debug_port_multi_count_input() {
        let mut bus = PortIoBus::with_debug_port_input(b'R');
        let io = input(DEBUG_PORT, 1, 2);

        assert!(matches!(
            bus.dispatch(&io),
            Err(Error::PortIo(PortIoError::UnsupportedDebugAccess {
                port: DEBUG_PORT,
                direction: 0,
                size: 1,
                count: 2,
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
