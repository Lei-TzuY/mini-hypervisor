use crate::error::PortIoError;
use crate::vcpu::{PortIoDirection, PortIoExit};

#[path = "pci/virtio.rs"]
pub mod virtio;

use virtio::VirtioRngPciFunction;

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
const PCI_CAP_ID_MSI: u8 = 0x05;
const VIRTIO_MSI_CAPABILITY_OFFSET: u8 = 0x74;
const VIRTIO_MSI_ADDRESS_OFFSET: u8 = 0x78;
const VIRTIO_MSI_DATA_OFFSET: u8 = 0x7c;
const PCI_MSI_ENABLE: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PciConfigService {
    Output,
    Input([u8; 4]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciMsiMessage {
    address: u32,
    data: u16,
}

impl PciMsiMessage {
    #[must_use]
    pub const fn address(self) -> u32 {
        self.address
    }

    #[must_use]
    pub const fn data(self) -> u16 {
        self.data
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PciMsiCapability {
    control: u16,
    address: u32,
    data: u16,
}

impl PciMsiCapability {
    const fn new() -> Self {
        Self {
            control: 0,
            address: 0,
            data: 0,
        }
    }

    const fn read_dword(&self, offset: u8) -> Option<u32> {
        match offset {
            VIRTIO_MSI_CAPABILITY_OFFSET => Some(u32::from_le_bytes([
                PCI_CAP_ID_MSI,
                0,
                self.control as u8,
                (self.control >> 8) as u8,
            ])),
            VIRTIO_MSI_ADDRESS_OFFSET => Some(self.address),
            VIRTIO_MSI_DATA_OFFSET => Some(self.data as u32),
            _ => None,
        }
    }

    fn write_dword(&mut self, offset: u8, value: u32) -> bool {
        match offset {
            VIRTIO_MSI_CAPABILITY_OFFSET => {
                if value as u16 != u16::from_le_bytes([PCI_CAP_ID_MSI, 0]) {
                    return false;
                }
                let control = (value >> 16) as u16;
                if control & !PCI_MSI_ENABLE != 0 {
                    return false;
                }
                self.control = control;
                true
            }
            VIRTIO_MSI_ADDRESS_OFFSET => {
                self.address = value;
                true
            }
            VIRTIO_MSI_DATA_OFFSET => {
                if value >> 16 != 0 {
                    return false;
                }
                self.data = value as u16;
                true
            }
            _ => false,
        }
    }

    const fn message(&self) -> Option<PciMsiMessage> {
        if self.control & PCI_MSI_ENABLE == 0 {
            None
        } else {
            Some(PciMsiMessage {
                address: self.address,
                data: self.data,
            })
        }
    }
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
struct VirtioRngPciEndpoint {
    function: VirtioRngPciFunction,
    msi: Option<PciMsiCapability>,
}

impl VirtioRngPciEndpoint {
    const fn new(function: VirtioRngPciFunction) -> Self {
        Self {
            function,
            msi: None,
        }
    }

    const fn with_msi(function: VirtioRngPciFunction) -> Self {
        Self {
            function,
            msi: Some(PciMsiCapability::new()),
        }
    }

    fn read_dword(&self, offset: u8) -> u32 {
        if let Some(value) = self.msi.as_ref().and_then(|msi| msi.read_dword(offset)) {
            return value;
        }
        let value = self.function.read_dword(offset);
        if offset == 0x64 && self.msi.is_some() {
            let mut bytes = value.to_le_bytes();
            bytes[1] = VIRTIO_MSI_CAPABILITY_OFFSET;
            u32::from_le_bytes(bytes)
        } else {
            value
        }
    }

    fn write_dword(&mut self, offset: u8, value: u32) -> bool {
        self.msi
            .as_mut()
            .is_some_and(|msi| msi.write_dword(offset, value))
    }

    fn msi_message(&self) -> Option<PciMsiMessage> {
        self.msi.as_ref().and_then(PciMsiCapability::message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PciFunction {
    Synthetic(SyntheticPciFunction),
    VirtioRng(VirtioRngPciEndpoint),
}

impl PciFunction {
    fn read_dword(&self, offset: u8) -> u32 {
        match self {
            Self::Synthetic(function) => function.read_dword(offset),
            Self::VirtioRng(function) => function.read_dword(offset),
        }
    }

    fn write_dword(&mut self, offset: u8, value: u32) -> bool {
        match self {
            Self::Synthetic(_) => false,
            Self::VirtioRng(function) => function.write_dword(offset, value),
        }
    }

    fn msi_message(&self) -> Option<PciMsiMessage> {
        match self {
            Self::Synthetic(_) => None,
            Self::VirtioRng(function) => function.msi_message(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PciConfigMechanism1 {
    address: u32,
    function: PciFunction,
}

impl PciConfigMechanism1 {
    #[must_use]
    pub const fn new(function: SyntheticPciFunction) -> Self {
        Self {
            address: 0,
            function: PciFunction::Synthetic(function),
        }
    }

    #[must_use]
    pub const fn with_virtio_rng(function: VirtioRngPciFunction) -> Self {
        Self {
            address: 0,
            function: PciFunction::VirtioRng(VirtioRngPciEndpoint::new(function)),
        }
    }

    #[must_use]
    pub const fn with_virtio_rng_msi(function: VirtioRngPciFunction) -> Self {
        Self {
            address: 0,
            function: PciFunction::VirtioRng(VirtioRngPciEndpoint::with_msi(function)),
        }
    }

    #[must_use]
    pub const fn handles_port(port: u16) -> bool {
        port == PCI_CONFIG_ADDRESS_PORT || port == PCI_CONFIG_DATA_PORT
    }

    #[must_use]
    pub fn virtio_rng_msi_message(&self) -> Option<PciMsiMessage> {
        self.function.msi_message()
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
            (PCI_CONFIG_DATA_PORT, PortIoDirection::In) => Ok(PciConfigService::Input(
                self.read_selected_dword().to_le_bytes(),
            )),
            (PCI_CONFIG_DATA_PORT, PortIoDirection::Out) => {
                let bytes: [u8; 4] = io.output_data().try_into().map_err(|_| unhandled(io))?;
                if self.write_selected_dword(u32::from_le_bytes(bytes)) {
                    Ok(PciConfigService::Output)
                } else {
                    Err(unhandled(io))
                }
            }
            _ => Err(unhandled(io)),
        }
    }

    fn selected_offset(&self) -> Option<u8> {
        if self.address & PCI_CONFIG_ENABLE == 0 {
            return None;
        }

        let bus = ((self.address >> 16) & 0xff) as u8;
        let device = ((self.address >> 11) & 0x1f) as u8;
        let function = ((self.address >> 8) & 0x07) as u8;
        if bus != SYNTHETIC_PCI_BUS
            || device != SYNTHETIC_PCI_DEVICE
            || function != SYNTHETIC_PCI_FUNCTION
        {
            return None;
        }
        Some((self.address & PCI_CONFIG_REGISTER_MASK) as u8)
    }

    fn read_selected_dword(&self) -> u32 {
        self.selected_offset()
            .map_or(u32::MAX, |offset| self.function.read_dword(offset))
    }

    fn write_selected_dword(&mut self, value: u32) -> bool {
        self.selected_offset()
            .is_some_and(|offset| self.function.write_dword(offset, value))
    }
}

#[must_use]
pub const fn config_selector(offset: u8) -> u32 {
    PCI_CONFIG_ENABLE
        | ((SYNTHETIC_PCI_BUS as u32) << 16)
        | ((SYNTHETIC_PCI_DEVICE as u32) << 11)
        | ((SYNTHETIC_PCI_FUNCTION as u32) << 8)
        | ((offset as u32) & PCI_CONFIG_REGISTER_MASK)
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
    use self::virtio::{VIRTIO_PCI_VENDOR_ID, VIRTIO_RNG_PCI_DEVICE_ID};
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

    fn write_config(config: &mut PciConfigMechanism1, offset: u8, value: u32) {
        assert_eq!(
            config.dispatch(&output(PCI_CONFIG_ADDRESS_PORT, config_selector(offset))),
            Ok(PciConfigService::Output)
        );
        assert_eq!(
            config.dispatch(&output(PCI_CONFIG_DATA_PORT, value)),
            Ok(PciConfigService::Output)
        );
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
    fn legacy_virtio_rng_capability_chain_remains_terminated() {
        let mut config = PciConfigMechanism1::with_virtio_rng(VirtioRngPciFunction::new(BAR0));
        assert_eq!(read_config(&mut config, 0x64).to_le_bytes(), [0x09, 0, 16, 3]);
        assert_eq!(config.virtio_rng_msi_message(), None);
    }

    #[test]
    fn virtio_rng_msi_capability_chain_ends_in_single_vector_32_bit_msi() {
        let mut config = PciConfigMechanism1::with_virtio_rng_msi(VirtioRngPciFunction::new(BAR0));
        assert_eq!(
            read_config(&mut config, 0x00),
            (u32::from(VIRTIO_RNG_PCI_DEVICE_ID) << 16) | u32::from(VIRTIO_PCI_VENDOR_ID)
        );
        assert_eq!(read_config(&mut config, 0x10), BAR0);
        assert_eq!(read_config(&mut config, 0x34) & 0xff, 0x40);
        assert_eq!(
            read_config(&mut config, 0x40).to_le_bytes(),
            [0x09, 0x50, 16, 1]
        );
        assert_eq!(
            read_config(&mut config, 0x50).to_le_bytes(),
            [0x09, 0x64, 20, 2]
        );
        assert_eq!(
            read_config(&mut config, 0x64).to_le_bytes(),
            [0x09, VIRTIO_MSI_CAPABILITY_OFFSET, 16, 3]
        );
        assert_eq!(
            read_config(&mut config, VIRTIO_MSI_CAPABILITY_OFFSET).to_le_bytes(),
            [PCI_CAP_ID_MSI, 0, 0, 0]
        );
    }

    #[test]
    fn virtio_rng_msi_message_is_guest_programmed_and_enable_gated() {
        let mut config = PciConfigMechanism1::with_virtio_rng_msi(VirtioRngPciFunction::new(BAR0));
        assert_eq!(config.virtio_rng_msi_message(), None);

        write_config(&mut config, VIRTIO_MSI_ADDRESS_OFFSET, 0xfee0_0000);
        write_config(&mut config, VIRTIO_MSI_DATA_OFFSET, 0x50);
        assert_eq!(config.virtio_rng_msi_message(), None);
        write_config(
            &mut config,
            VIRTIO_MSI_CAPABILITY_OFFSET,
            u32::from(u16::from_le_bytes([PCI_CAP_ID_MSI, 0])) | (u32::from(PCI_MSI_ENABLE) << 16),
        );

        assert_eq!(
            read_config(&mut config, VIRTIO_MSI_ADDRESS_OFFSET),
            0xfee0_0000
        );
        assert_eq!(read_config(&mut config, VIRTIO_MSI_DATA_OFFSET), 0x50);
        assert_eq!(
            config.virtio_rng_msi_message(),
            Some(PciMsiMessage {
                address: 0xfee0_0000,
                data: 0x50,
            })
        );

        write_config(
            &mut config,
            VIRTIO_MSI_CAPABILITY_OFFSET,
            u32::from(u16::from_le_bytes([PCI_CAP_ID_MSI, 0])),
        );
        assert_eq!(config.virtio_rng_msi_message(), None);
    }

    #[test]
    fn rejects_absent_or_read_only_config_writes() {
        let mut synthetic = PciConfigMechanism1::new(SyntheticPciFunction::new(BAR0));
        synthetic
            .dispatch(&output(PCI_CONFIG_ADDRESS_PORT, config_selector(0x10)))
            .unwrap();
        assert!(matches!(
            synthetic.dispatch(&output(PCI_CONFIG_DATA_PORT, 1)),
            Err(PortIoError::UnhandledPort { .. })
        ));

        let mut virtio = PciConfigMechanism1::with_virtio_rng(VirtioRngPciFunction::new(BAR0));
        virtio
            .dispatch(&output(PCI_CONFIG_ADDRESS_PORT, config_selector(0x10)))
            .unwrap();
        assert!(matches!(
            virtio.dispatch(&output(PCI_CONFIG_DATA_PORT, 1)),
            Err(PortIoError::UnhandledPort { .. })
        ));
    }

    #[test]
    fn rejects_unknown_msi_control_bits_without_mutating_enable_state() {
        let mut config = PciConfigMechanism1::with_virtio_rng_msi(VirtioRngPciFunction::new(BAR0));
        write_config(&mut config, VIRTIO_MSI_ADDRESS_OFFSET, 0xfee0_0000);
        write_config(&mut config, VIRTIO_MSI_DATA_OFFSET, 0x50);
        config
            .dispatch(&output(
                PCI_CONFIG_ADDRESS_PORT,
                config_selector(VIRTIO_MSI_CAPABILITY_OFFSET),
            ))
            .unwrap();
        assert!(matches!(
            config.dispatch(&output(PCI_CONFIG_DATA_PORT, 0x0002_0005)),
            Err(PortIoError::UnhandledPort { .. })
        ));
        assert_eq!(config.virtio_rng_msi_message(), None);
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
    fn rejects_non_dword_config_cycles() {
        let mut config = PciConfigMechanism1::new(SyntheticPciFunction::new(BAR0));
        let narrow = PortIoExit::new(PortIoDirection::In, 2, PCI_CONFIG_DATA_PORT, 1, Vec::new());
        assert!(matches!(
            config.dispatch(&narrow),
            Err(PortIoError::UnhandledPort { .. })
        ));
    }
}
