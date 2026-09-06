#[path = "pci.rs"]
pub mod pci;
pub mod pci_fixture;
pub mod two_vcpu_ap_local_timer_fixture;
pub mod two_vcpu_fixture;
pub mod two_vcpu_guest_ipi_fixture;
pub mod two_vcpu_init_sipi_fixture;
pub mod two_vcpu_sipi_work_dispatch_fixture;
pub mod two_vcpu_targeted_msi_fixture;
pub mod two_vcpu_tlb_shootdown_fixture;
pub mod two_vcpu_work_dispatch_fixture;
pub mod virtio_blk_completion_interrupt_fixture;
pub mod virtio_blk_fixture;
pub mod virtio_blk_multi_sector_fixture;
pub mod virtio_rng_completion_interrupt_fixture;
pub mod virtio_rng_fixture;
pub mod virtio_rng_msi_completion_fixture;

use crate::error::{Error, PortIoError};
use crate::vcpu::{PortIoDirection, PortIoExit};
use pci::{PciConfigMechanism1, PciConfigService, PciMsiMessage};

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

    #[must_use]
    pub fn virtio_rng_msi_message(&self) -> Option<PciMsiMessage> {
        self.pci_config
            .as_ref()
            .and_then(PciConfigMechanism1::virtio_rng_msi_message)
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

#[derive(Debug, Clone)]
struct DebugPort {
    input_byte: u8,
    bytes: Vec<u8>,
}

impl Default for DebugPort {
    fn default() -> Self {
        Self {
            input_byte: b'R',
            bytes: Vec::new(),
        }
    }
}

impl DebugPort {
    fn handle(&mut self, io: &PortIoExit) -> Result<PortIoService, PortIoError> {
        if io.size() != 1 || io.count() != 1 {
            return Err(PortIoError::UnsupportedAccessShape {
                port: io.port(),
                direction: io.direction().raw(),
                size: io.size(),
                count: io.count(),
            });
        }

        match io.direction() {
            PortIoDirection::Out => {
                self.bytes.extend_from_slice(io.output_data());
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
    use super::*;
    use crate::vcpu::PortIoDirection;

    #[test]
    fn empty_bus_rejects_unhandled_port() {
        let mut bus = PortIoBus::empty();
        let io = PortIoExit::new_for_test(0x1234, PortIoDirection::Out, 1, 1, vec![0x41]);
        let error = bus.dispatch(&io).unwrap_err();
        assert!(matches!(
            error,
            Error::PortIo(PortIoError::UnhandledPort { port: 0x1234, .. })
        ));
    }

    #[test]
    fn debug_port_captures_one_byte_outputs() {
        let mut bus = PortIoBus::with_debug_port();
        for byte in b"KVM" {
            let io = PortIoExit::new_for_test(
                DEBUG_PORT,
                PortIoDirection::Out,
                1,
                1,
                vec![*byte],
            );
            assert_eq!(bus.dispatch(&io).unwrap(), PortIoService::Output);
        }
        assert_eq!(bus.debug_output(), Some(&b"KVM"[..]));
    }

    #[test]
    fn debug_port_supplies_configured_one_byte_input() {
        let mut bus = PortIoBus::with_debug_port_input(b'Q');
        let io = PortIoExit::new_for_test(DEBUG_PORT, PortIoDirection::In, 1, 1, Vec::new());
        assert_eq!(
            bus.dispatch(&io).unwrap(),
            PortIoService::Input(vec![b'Q'])
        );
    }

    #[test]
    fn debug_port_rejects_wide_access() {
        let mut bus = PortIoBus::with_debug_port();
        let io = PortIoExit::new_for_test(
            DEBUG_PORT,
            PortIoDirection::Out,
            2,
            1,
            vec![b'K', b'V'],
        );
        let error = bus.dispatch(&io).unwrap_err();
        assert!(matches!(
            error,
            Error::PortIo(PortIoError::UnsupportedAccessShape { size: 2, .. })
        ));
    }
}
