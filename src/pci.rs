use crate::error::PortIoError;
use crate::vcpu::{PortIoDirection, PortIoExit};

pub const PCI_CONFIG_ADDRESS_PORT: u16 = 0x0cf8;
pub const PCI_CONFIG_DATA_PORT: u16 = 0x0cfc;
pub const SYNTHETIC_PCI_BUS: u8 = 0;
pub const SYNTHETIC_PCI_DEVICE: u8 = 1;
pub const SYNTHETIC_PCI_FUNCTION: u8 = 0;
pub const SYNTHETIC_PCI_VENDOR_ID: u16 = 0xcafe;
pub const SYNTHETIC_PCI_DEVICE_ID: u16 = 0x0001;
pub const SYNTHETIC_PCI_CLASS_CODE: u8 = 0xff;
pub const SYNTHETIC_PCI_REVISION: u8 = 1;

const PCI_CONFIG_ENABLE: u32 = 1 << 31;
const PCI_CONFIG_REGISTER_MASK: u32 = 0xfc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PciConfigService {
    Output,
    Input([u8; 4]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticPciFunction {
    bar0: u32,
}

impl SyntheticPciFunction {
    #[must_use]
    pub const fn new(bar0: u32) -> Self {
        Self {
            bar0: bar0 & 0xffff_fff0,
        }
    }

    #[must_use]
    pub const fn bar0(&self) -> u32 {
        self.bar0
    }

    fn read_dword(&self, offset: u8) -> u32 {
        match offset {
            0x00 => (u32::from(SYNTHETIC_PCI_DEVICE_ID) << 16) | u32::from(SYNTHETIC_PCI_VENDOR_ID),
            0x04 => 0,
            0x08 => (u32::from(SYNTHETIC_PCI_CLASS_CODE) << 24) | u32::from(SYNTHETIC_PCI_REVISION),
            0x0c => 0,
            0x10 => self.bar0,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PciConfigMechanism1 {
    address: u32,
    function: SyntheticPciFunction,
}

impl PciConfigMechanism1 {
    #[must_use]
    pub const fn new(function: SyntheticPciFunction) -> Self {
        Self {
            address: 0,
            function,
        }
    }

    #[must_use]
    pub const fn handles_port(port: u16) -> bool {
        port == PCI_CONFIG_ADDRESS_PORT || port == PCI_CONFIG_DATA_PORT
    }

    pub fn dispatch(&mut self, io: &PortIoExit) -> Result<PciConfigService, PortIoError> {
        if io.size() != 4 || io.count() != 1 {
            return Err(unhandled(io));
        }

        match (io.port(), io.direction()) {
            (PCI_CONFIG_ADDRESS_PORT, PortIoDirection::Out) => {
                let bytes: [u8; 4] = io.output_data().try_into().map_err(|_| unhandled(io))?;
                self.address = u32::from_le_bytes(bytes);
                Ok(PciConfigService::Output)
            }
            (PCI_CONFIG_ADDRESS_PORT, PortIoDirection::In) => {
                Ok(PciConfigService::Input(self.address.to_le_bytes()))
            }
            (PCI_CONFIG_DATA_PORT, PortIoDirection::In) => {
                Ok(PciConfigService::Input(self.read_selected_dword().to_le_bytes()))
            }
            (PCI_CONFIG_DATA_PORT, PortIoDirection::Out) => Err(unhandled(io)),
            _ => Err(unhandled(io)),
        }
    }

    fn read_selected_dword(&self) -> u32 {
        if self.address & PCI_CONFIG_ENABLE == 0 {
            return u32::MAX;
        }

        let bus = ((self.address >> 16) & 0xff) as u8;
        let device = ((self.address >> 11) & 0x1f) as u8;
        let function = ((self.address >> 8) & 0x07) as u8;
        if bus != SYNTHETIC_PCI_BUS
            || device != SYNTHETIC_PCI_DEVICE
            || function != SYNTHETIC_PCI_FUNCTION
        {
            return u32::MAX;
        }

        self.function
            .read_dword((self.address & PCI_CONFIG_REGISTER_MASK) as u8)
    }
}

#[must_use]
pub const fn config_selector(offset: u8) -> u32 {
    PCI_CONFIG_ENABLE
        | (u32::from(SYNTHETIC_PCI_BUS) << 16)
        | (u32::from(SYNTHETIC_PCI_DEVICE) << 11)
        | (u32::from(SYNTHETIC_PCI_FUNCTION) << 8)
        | (u32::from(offset) & PCI_CONFIG_REGISTER_MASK)
}

fn unhandled(io: &PortIoExit) -> PortIoError {
    PortIoError::UnhandledPort {
        port: io.port(),
        direction: io.direction().raw(),
        size: io.size(),
        count: io.count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR0: u32 = 0x1000_0000;

    fn output(port: u16, value: u32) -> PortIoExit {
        PortIoExit::new(
            PortIoDirection::Out,
            4,
            port,
            1,
            value.to_le_bytes().to_vec(),
        )
    }

    fn input(port: u16) -> PortIoExit {
        PortIoExit::new(PortIoDirection::In, 4, port, 1, Vec::new())
    }

    fn read_config(config: &mut PciConfigMechanism1, offset: u8) -> u32 {
        assert_eq!(
            config.dispatch(&output(PCI_CONFIG_ADDRESS_PORT, config_selector(offset))),
            Ok(PciConfigService::Output)
        );
        match config.dispatch(&input(PCI_CONFIG_DATA_PORT)).unwrap() {
            PciConfigService::Input(bytes) => u32::from_le_bytes(bytes),
            PciConfigService::Output => panic!("config read returned output service"),
        }
    }

    #[test]
    fn exposes_identity_class_and_bar0() {
        let mut config = PciConfigMechanism1::new(SyntheticPciFunction::new(BAR0));

        assert_eq!(
            read_config(&mut config, 0x00),
            (u32::from(SYNTHETIC_PCI_DEVICE_ID) << 16) | u32::from(SYNTHETIC_PCI_VENDOR_ID)
        );
        assert_eq!(
            read_config(&mut config, 0x08),
            (u32::from(SYNTHETIC_PCI_CLASS_CODE) << 24) | u32::from(SYNTHETIC_PCI_REVISION)
        );
        assert_eq!(read_config(&mut config, 0x10), BAR0);
    }

    #[test]
    fn absent_function_reads_all_ones() {
        let mut config = PciConfigMechanism1::new(SyntheticPciFunction::new(BAR0));
        let absent_selector = PCI_CONFIG_ENABLE | (2 << 11);

        config
            .dispatch(&output(PCI_CONFIG_ADDRESS_PORT, absent_selector))
            .unwrap();
        assert_eq!(
            config.dispatch(&input(PCI_CONFIG_DATA_PORT)),
            Ok(PciConfigService::Input(u32::MAX.to_le_bytes()))
        );
    }

    #[test]
    fn disabled_config_address_reads_all_ones() {
        let mut config = PciConfigMechanism1::new(SyntheticPciFunction::new(BAR0));
        assert_eq!(
            config.dispatch(&input(PCI_CONFIG_DATA_PORT)),
            Ok(PciConfigService::Input(u32::MAX.to_le_bytes()))
        );
    }

    #[test]
    fn rejects_data_writes_and_non_dword_cycles() {
        let mut config = PciConfigMechanism1::new(SyntheticPciFunction::new(BAR0));
        assert!(matches!(
            config.dispatch(&output(PCI_CONFIG_DATA_PORT, 1)),
            Err(PortIoError::UnhandledPort { .. })
        ));

        let narrow = PortIoExit::new(PortIoDirection::In, 2, PCI_CONFIG_DATA_PORT, 1, Vec::new());
        assert!(matches!(
            config.dispatch(&narrow),
            Err(PortIoError::UnhandledPort { .. })
        ));
    }
}
