use super::virtio::{
    VIRTIO_F_VERSION_1, VIRTIO_ISR_LENGTH, VIRTIO_ISR_OFFSET, VIRTIO_ISR_QUEUE_INTERRUPT,
    VIRTIO_NOTIFY_LENGTH, VIRTIO_NOTIFY_OFFSET, VIRTIO_NOTIFY_OFF_MULTIPLIER,
    VIRTIO_PCI_CAP_COMMON_CFG, VIRTIO_PCI_CAP_ISR_CFG, VIRTIO_PCI_CAP_NOTIFY_CFG,
    VIRTIO_PCI_CAP_VENDOR_ID, VIRTIO_PCI_VENDOR_ID, VIRTIO_QUEUE_MAX_SIZE,
    VIRTIO_STATUS_ACKNOWLEDGE, VIRTIO_STATUS_DRIVER, VIRTIO_STATUS_DRIVER_OK, VIRTIO_STATUS_FAILED,
    VIRTIO_STATUS_FEATURES_OK,
};
use crate::error::Error;
use crate::memory::{GuestMemory, GuestPhysAddr};
use std::fmt;

pub const VIRTIO_BLK_DEVICE_TYPE: u16 = 2;
pub const VIRTIO_BLK_PCI_DEVICE_ID: u16 = 0x1040 + VIRTIO_BLK_DEVICE_TYPE;
pub const VIRTIO_BLK_PCI_REVISION: u8 = 1;
pub const VIRTIO_BLK_PCI_CLASS_CODE: u8 = 0x01;
pub const VIRTIO_BLK_BAR_SIZE: u32 = 0x1000;
pub const VIRTIO_BLK_QUEUE_INDEX: u16 = 0;
pub const VIRTIO_BLK_CAPACITY_SECTORS: u64 = 1;
pub const VIRTIO_BLK_SECTOR_SIZE: usize = 512;
pub const VIRTIO_BLK_CONFIG_OFFSET: u64 = 0x300;
pub const VIRTIO_BLK_CONFIG_LENGTH: u32 = 8;
pub const VIRTIO_BLK_T_IN: u32 = 0;
pub const VIRTIO_BLK_S_OK: u8 = 0;
pub const VIRTQ_DESC_F_NEXT: u16 = 1;
pub const VIRTQ_DESC_F_WRITE: u16 = 2;

const COMMON_DEVICE_FEATURE_SELECT: u64 = 0x00;
const COMMON_DEVICE_FEATURE: u64 = 0x04;
const COMMON_DRIVER_FEATURE_SELECT: u64 = 0x08;
const COMMON_DRIVER_FEATURE: u64 = 0x0c;
const COMMON_CONFIG_MSIX_VECTOR: u64 = 0x10;
const COMMON_NUM_QUEUES: u64 = 0x12;
const COMMON_DEVICE_STATUS: u64 = 0x14;
const COMMON_CONFIG_GENERATION: u64 = 0x15;
const COMMON_QUEUE_SELECT: u64 = 0x16;
const COMMON_QUEUE_SIZE: u64 = 0x18;
const COMMON_QUEUE_MSIX_VECTOR: u64 = 0x1a;
const COMMON_QUEUE_ENABLE: u64 = 0x1c;
const COMMON_QUEUE_NOTIFY_OFF: u64 = 0x1e;
const COMMON_QUEUE_DESC: u64 = 0x20;
const COMMON_QUEUE_DESC_HIGH: u64 = 0x24;
const COMMON_QUEUE_DRIVER: u64 = 0x28;
const COMMON_QUEUE_DRIVER_HIGH: u64 = 0x2c;
const COMMON_QUEUE_DEVICE: u64 = 0x30;
const COMMON_QUEUE_DEVICE_HIGH: u64 = 0x34;

const PCI_STATUS_CAPABILITIES_LIST: u16 = 1 << 4;
const VIRTIO_CAP_COMMON: u8 = 0x40;
const VIRTIO_CAP_NOTIFY: u8 = 0x50;
const VIRTIO_CAP_ISR: u8 = 0x64;
const VIRTIO_CAP_DEVICE: u8 = 0x74;
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;
const VIRTIO_STATUS_KNOWN: u8 = VIRTIO_STATUS_ACKNOWLEDGE
    | VIRTIO_STATUS_DRIVER
    | VIRTIO_STATUS_DRIVER_OK
    | VIRTIO_STATUS_FEATURES_OK
    | VIRTIO_STATUS_FAILED;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtioBlkError {
    UnsupportedRegisterAccess {
        offset: u64,
        length: usize,
        write: bool,
    },
    InvalidRegisterPayload {
        offset: u64,
        expected: usize,
        actual: usize,
    },
    InvalidDeviceStatus {
        current: u8,
        requested: u8,
    },
    UnsupportedQueue {
        queue: u16,
    },
    InvalidQueueSize {
        size: u16,
        maximum: u16,
    },
    QueueConfigurationLocked,
    QueueNotReady,
    UnexpectedQueueNotification {
        queue: u16,
    },
    UnexpectedAvailIndex {
        expected: u16,
        actual: u16,
    },
    UnexpectedUsedIndex {
        expected: u16,
        actual: u16,
    },
    DescriptorIndexOutOfRange {
        index: u16,
        queue_size: u16,
    },
    InvalidDescriptorFlags {
        index: u16,
        expected: u16,
        actual: u16,
    },
    DescriptorTooSmall {
        index: u16,
        length: u32,
        required: u32,
    },
    DescriptorChainCycle {
        index: u16,
    },
    InvalidRequestType {
        request_type: u32,
    },
    InvalidRequestReserved {
        reserved: u32,
    },
    SectorOutOfRange {
        sector: u64,
        capacity: u64,
    },
    AddressOverflow {
        base: u64,
        offset: u64,
    },
}

impl fmt::Display for VirtioBlkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedRegisterAccess {
                offset,
                length,
                write,
            } => write!(
                f,
                "unsupported virtio-blk {} access at BAR offset {offset:#x} with length {length}",
                if *write { "write" } else { "read" }
            ),
            Self::InvalidRegisterPayload {
                offset,
                expected,
                actual,
            } => write!(
                f,
                "invalid virtio-blk write payload at BAR offset {offset:#x}: expected {expected} bytes, got {actual}"
            ),
            Self::InvalidDeviceStatus { current, requested } => write!(
                f,
                "invalid virtio-blk device-status transition {current:#x} -> {requested:#x}"
            ),
            Self::UnsupportedQueue { queue } => {
                write!(f, "virtio-blk exposes only requestq 0, not queue {queue}")
            }
            Self::InvalidQueueSize { size, maximum } => write!(
                f,
                "invalid virtio-blk queue size {size}; maximum power-of-two size is {maximum}"
            ),
            Self::QueueConfigurationLocked => {
                write!(f, "virtio-blk queue configuration is locked")
            }
            Self::QueueNotReady => {
                write!(f, "virtio-blk requestq is not fully negotiated and enabled")
            }
            Self::UnexpectedQueueNotification { queue } => {
                write!(f, "virtio-blk received notification for unsupported queue {queue}")
            }
            Self::UnexpectedAvailIndex { expected, actual } => {
                write!(f, "virtio-blk expected avail.idx {expected}, got {actual}")
            }
            Self::UnexpectedUsedIndex { expected, actual } => {
                write!(f, "virtio-blk expected used.idx {expected}, got {actual}")
            }
            Self::DescriptorIndexOutOfRange { index, queue_size } => write!(
                f,
                "virtio-blk descriptor index {index} is outside queue size {queue_size}"
            ),
            Self::InvalidDescriptorFlags {
                index,
                expected,
                actual,
            } => write!(
                f,
                "virtio-blk descriptor {index} flags {actual:#x} do not match required {expected:#x}"
            ),
            Self::DescriptorTooSmall {
                index,
                length,
                required,
            } => write!(
                f,
                "virtio-blk descriptor {index} length {length} is smaller than required {required}"
            ),
            Self::DescriptorChainCycle { index } => {
                write!(f, "virtio-blk descriptor chain revisits descriptor {index}")
            }
            Self::InvalidRequestType { request_type } => write!(
                f,
                "virtio-blk first slice accepts only VIRTIO_BLK_T_IN (0), got {request_type}"
            ),
            Self::InvalidRequestReserved { reserved } => write!(
                f,
                "virtio-blk request reserved field must be zero, got {reserved:#x}"
            ),
            Self::SectorOutOfRange { sector, capacity } => write!(
                f,
                "virtio-blk sector {sector} is outside capacity {capacity} sectors"
            ),
            Self::AddressOverflow { base, offset } => write!(
                f,
                "virtio-blk guest address arithmetic overflows: base={base:#x}, offset={offset:#x}"
            ),
        }
    }
}

impl std::error::Error for VirtioBlkError {}

#[derive(Debug)]
pub enum VirtioBlkProcessError {
    Device(VirtioBlkError),
    Memory(Error),
}

impl fmt::Display for VirtioBlkProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(error) => error.fmt(f),
            Self::Memory(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for VirtioBlkProcessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Device(error) => Some(error),
            Self::Memory(error) => Some(error),
        }
    }
}

impl From<VirtioBlkError> for VirtioBlkProcessError {
    fn from(value: VirtioBlkError) -> Self {
        Self::Device(value)
    }
}

impl From<Error> for VirtioBlkProcessError {
    fn from(value: Error) -> Self {
        Self::Memory(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioBlkQueueCompletion {
    descriptor_id: u32,
    length: u32,
    sector: u64,
}

impl VirtioBlkQueueCompletion {
    #[must_use]
    pub const fn descriptor_id(self) -> u32 {
        self.descriptor_id
    }

    #[must_use]
    pub const fn length(self) -> u32 {
        self.length
    }

    #[must_use]
    pub const fn sector(self) -> u64 {
        self.sector
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioBlkPciFunction {
    bar0: u32,
}

impl VirtioBlkPciFunction {
    #[must_use]
    pub const fn new(bar0: u32) -> Self {
        Self {
            bar0: bar0 & 0xffff_f000,
        }
    }

    #[must_use]
    pub const fn bar0(&self) -> u32 {
        self.bar0
    }

    pub(super) fn read_dword(&self, offset: u8) -> u32 {
        match offset {
            0x00 => (u32::from(VIRTIO_BLK_PCI_DEVICE_ID) << 16) | u32::from(VIRTIO_PCI_VENDOR_ID),
            0x04 => u32::from(PCI_STATUS_CAPABILITIES_LIST) << 16,
            0x08 => {
                (u32::from(VIRTIO_BLK_PCI_CLASS_CODE) << 24) | u32::from(VIRTIO_BLK_PCI_REVISION)
            }
            0x0c => 0,
            0x10 => self.bar0,
            0x2c => (0x0002_u32 << 16) | u32::from(VIRTIO_PCI_VENDOR_ID),
            0x34 => u32::from(VIRTIO_CAP_COMMON),
            0x40 => capability_header(VIRTIO_CAP_NOTIFY, 16, VIRTIO_PCI_CAP_COMMON_CFG),
            0x44 => 0,
            0x48 => 0,
            0x4c => 0x38,
            0x50 => capability_header(VIRTIO_CAP_ISR, 20, VIRTIO_PCI_CAP_NOTIFY_CFG),
            0x54 => 0,
            0x58 => VIRTIO_NOTIFY_OFFSET as u32,
            0x5c => VIRTIO_NOTIFY_LENGTH,
            0x60 => VIRTIO_NOTIFY_OFF_MULTIPLIER,
            0x64 => capability_header(VIRTIO_CAP_DEVICE, 16, VIRTIO_PCI_CAP_ISR_CFG),
            0x68 => 0,
            0x6c => VIRTIO_ISR_OFFSET as u32,
            0x70 => VIRTIO_ISR_LENGTH,
            0x74 => capability_header(0, 16, VIRTIO_PCI_CAP_DEVICE_CFG),
            0x78 => 0,
            0x7c => VIRTIO_BLK_CONFIG_OFFSET as u32,
            0x80 => VIRTIO_BLK_CONFIG_LENGTH,
            _ => 0,
        }
    }
}

const fn capability_header(next: u8, length: u8, cfg_type: u8) -> u32 {
    u32::from_le_bytes([VIRTIO_PCI_CAP_VENDOR_ID, next, length, cfg_type])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioBlkEvent {
    QueueNotified { queue: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioBlkDevice {
    bar0: u64,
    device_feature_select: u32,
    driver_feature_select: u32,
    driver_features: u64,
    status: u8,
    queue_select: u16,
    queue_size: u16,
    queue_enabled: bool,
    queue_desc: u64,
    queue_driver: u64,
    queue_device: u64,
    notify_pending: bool,
    last_avail_idx: u16,
    last_used_idx: u16,
    isr_status: u8,
    sector0: [u8; VIRTIO_BLK_SECTOR_SIZE],
}

impl VirtioBlkDevice {
    #[must_use]
    pub fn new(bar0: u64) -> Self {
        Self {
            bar0,
            device_feature_select: 0,
            driver_feature_select: 0,
            driver_features: 0,
            status: 0,
            queue_select: 0,
            queue_size: VIRTIO_QUEUE_MAX_SIZE,
            queue_enabled: false,
            queue_desc: 0,
            queue_driver: 0,
            queue_device: 0,
            notify_pending: false,
            last_avail_idx: 0,
            last_used_idx: 0,
            isr_status: 0,
            sector0: deterministic_sector(),
        }
    }

    #[must_use]
    pub const fn bar0(&self) -> u64 {
        self.bar0
    }

    #[must_use]
    pub const fn status(&self) -> u8 {
        self.status
    }

    #[must_use]
    pub const fn driver_features(&self) -> u64 {
        self.driver_features
    }

    #[must_use]
    pub const fn queue_enabled(&self) -> bool {
        self.queue_enabled
    }

    #[must_use]
    pub const fn isr_status(&self) -> u8 {
        self.isr_status
    }

    pub fn read(&mut self, offset: u64, length: usize) -> Result<Vec<u8>, VirtioBlkError> {
        let bytes = match (offset, length) {
            (COMMON_DEVICE_FEATURE_SELECT, 4) => self.device_feature_select.to_le_bytes().to_vec(),
            (COMMON_DEVICE_FEATURE, 4) => self.device_feature_word().to_le_bytes().to_vec(),
            (COMMON_DRIVER_FEATURE_SELECT, 4) => self.driver_feature_select.to_le_bytes().to_vec(),
            (COMMON_DRIVER_FEATURE, 4) => self.driver_feature_word().to_le_bytes().to_vec(),
            (COMMON_CONFIG_MSIX_VECTOR, 2) => u16::MAX.to_le_bytes().to_vec(),
            (COMMON_NUM_QUEUES, 2) => 1_u16.to_le_bytes().to_vec(),
            (COMMON_DEVICE_STATUS, 1) => vec![self.status],
            (COMMON_CONFIG_GENERATION, 1) => vec![0],
            (COMMON_QUEUE_SELECT, 2) => self.queue_select.to_le_bytes().to_vec(),
            (COMMON_QUEUE_SIZE, 2) => self.selected_queue_size().to_le_bytes().to_vec(),
            (COMMON_QUEUE_MSIX_VECTOR, 2) => u16::MAX.to_le_bytes().to_vec(),
            (COMMON_QUEUE_ENABLE, 2) => u16::from(self.queue_enabled).to_le_bytes().to_vec(),
            (COMMON_QUEUE_NOTIFY_OFF, 2) => 0_u16.to_le_bytes().to_vec(),
            (COMMON_QUEUE_DESC, 4) => (self.queue_desc as u32).to_le_bytes().to_vec(),
            (COMMON_QUEUE_DESC_HIGH, 4) => ((self.queue_desc >> 32) as u32).to_le_bytes().to_vec(),
            (COMMON_QUEUE_DRIVER, 4) => (self.queue_driver as u32).to_le_bytes().to_vec(),
            (COMMON_QUEUE_DRIVER_HIGH, 4) => {
                ((self.queue_driver >> 32) as u32).to_le_bytes().to_vec()
            }
            (COMMON_QUEUE_DEVICE, 4) => (self.queue_device as u32).to_le_bytes().to_vec(),
            (COMMON_QUEUE_DEVICE_HIGH, 4) => {
                ((self.queue_device >> 32) as u32).to_le_bytes().to_vec()
            }
            (VIRTIO_ISR_OFFSET, 1) => {
                let status = self.isr_status;
                self.isr_status = 0;
                vec![status]
            }
            (VIRTIO_BLK_CONFIG_OFFSET, 8) => VIRTIO_BLK_CAPACITY_SECTORS.to_le_bytes().to_vec(),
            _ => {
                return Err(VirtioBlkError::UnsupportedRegisterAccess {
                    offset,
                    length,
                    write: false,
                })
            }
        };
        Ok(bytes)
    }

    pub fn write(
        &mut self,
        offset: u64,
        payload: &[u8],
    ) -> Result<Option<VirtioBlkEvent>, VirtioBlkError> {
        match (offset, payload.len()) {
            (COMMON_DEVICE_FEATURE_SELECT, 4) => {
                self.device_feature_select = read_u32(offset, payload)?
            }
            (COMMON_DRIVER_FEATURE_SELECT, 4) => {
                self.driver_feature_select = read_u32(offset, payload)?
            }
            (COMMON_DRIVER_FEATURE, 4) => {
                self.write_driver_feature_word(read_u32(offset, payload)?)?
            }
            (COMMON_DEVICE_STATUS, 1) => self.write_status(payload[0])?,
            (COMMON_QUEUE_SELECT, 2) => self.queue_select = read_u16(offset, payload)?,
            (COMMON_QUEUE_SIZE, 2) => self.write_queue_size(read_u16(offset, payload)?)?,
            (COMMON_QUEUE_ENABLE, 2) => self.write_queue_enable(read_u16(offset, payload)?)?,
            (COMMON_QUEUE_DESC, 4) => {
                self.ensure_queue_unlocked()?;
                self.queue_desc = replace_low_u32(self.queue_desc, read_u32(offset, payload)?);
            }
            (COMMON_QUEUE_DESC_HIGH, 4) => {
                self.ensure_queue_unlocked()?;
                self.queue_desc = replace_high_u32(self.queue_desc, read_u32(offset, payload)?);
            }
            (COMMON_QUEUE_DRIVER, 4) => {
                self.ensure_queue_unlocked()?;
                self.queue_driver = replace_low_u32(self.queue_driver, read_u32(offset, payload)?);
            }
            (COMMON_QUEUE_DRIVER_HIGH, 4) => {
                self.ensure_queue_unlocked()?;
                self.queue_driver = replace_high_u32(self.queue_driver, read_u32(offset, payload)?);
            }
            (COMMON_QUEUE_DEVICE, 4) => {
                self.ensure_queue_unlocked()?;
                self.queue_device = replace_low_u32(self.queue_device, read_u32(offset, payload)?);
            }
            (COMMON_QUEUE_DEVICE_HIGH, 4) => {
                self.ensure_queue_unlocked()?;
                self.queue_device = replace_high_u32(self.queue_device, read_u32(offset, payload)?);
            }
            (VIRTIO_NOTIFY_OFFSET, 2) => {
                let queue = read_u16(offset, payload)?;
                if queue != VIRTIO_BLK_QUEUE_INDEX {
                    return Err(VirtioBlkError::UnexpectedQueueNotification { queue });
                }
                self.ensure_queue_ready()?;
                self.notify_pending = true;
                return Ok(Some(VirtioBlkEvent::QueueNotified { queue }));
            }
            _ => {
                return Err(VirtioBlkError::UnsupportedRegisterAccess {
                    offset,
                    length: payload.len(),
                    write: true,
                })
            }
        }
        Ok(None)
    }

    pub fn process_notified_queue(
        &mut self,
        memory: &mut GuestMemory,
    ) -> Result<VirtioBlkQueueCompletion, VirtioBlkProcessError> {
        self.ensure_queue_ready()?;
        if !self.notify_pending {
            return Err(VirtioBlkError::QueueNotReady.into());
        }

        let avail_idx = read_guest_u16(memory, checked_add(self.queue_driver, 2)?)?;
        let expected_avail = self.last_avail_idx.wrapping_add(1);
        if avail_idx != expected_avail {
            return Err(VirtioBlkError::UnexpectedAvailIndex {
                expected: expected_avail,
                actual: avail_idx,
            }
            .into());
        }
        let slot = self.last_avail_idx % self.queue_size;
        let head = read_guest_u16(
            memory,
            checked_add(self.queue_driver, 4 + 2 * u64::from(slot))?,
        )?;
        self.ensure_descriptor_index(head)?;

        let header = self.read_descriptor(memory, head)?;
        self.require_flags(head, header.flags, VIRTQ_DESC_F_NEXT)?;
        self.require_length(head, header.length, 16)?;
        let data_index = header.next;
        self.ensure_descriptor_index(data_index)?;
        if data_index == head {
            return Err(VirtioBlkError::DescriptorChainCycle { index: data_index }.into());
        }

        let data = self.read_descriptor(memory, data_index)?;
        self.require_flags(
            data_index,
            data.flags,
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
        )?;
        self.require_length(data_index, data.length, VIRTIO_BLK_SECTOR_SIZE as u32)?;
        let status_index = data.next;
        self.ensure_descriptor_index(status_index)?;
        if status_index == head || status_index == data_index {
            return Err(VirtioBlkError::DescriptorChainCycle {
                index: status_index,
            }
            .into());
        }

        let status = self.read_descriptor(memory, status_index)?;
        self.require_flags(status_index, status.flags, VIRTQ_DESC_F_WRITE)?;
        self.require_length(status_index, status.length, 1)?;

        let mut request = [0_u8; 16];
        memory.read(GuestPhysAddr::new(header.address), &mut request)?;
        let request_type = u32::from_le_bytes(request[0..4].try_into().unwrap());
        let reserved = u32::from_le_bytes(request[4..8].try_into().unwrap());
        let sector = u64::from_le_bytes(request[8..16].try_into().unwrap());
        if request_type != VIRTIO_BLK_T_IN {
            return Err(VirtioBlkError::InvalidRequestType { request_type }.into());
        }
        if reserved != 0 {
            return Err(VirtioBlkError::InvalidRequestReserved { reserved }.into());
        }
        if sector >= VIRTIO_BLK_CAPACITY_SECTORS {
            return Err(VirtioBlkError::SectorOutOfRange {
                sector,
                capacity: VIRTIO_BLK_CAPACITY_SECTORS,
            }
            .into());
        }

        let used_idx = read_guest_u16(memory, checked_add(self.queue_device, 2)?)?;
        if used_idx != self.last_used_idx {
            return Err(VirtioBlkError::UnexpectedUsedIndex {
                expected: self.last_used_idx,
                actual: used_idx,
            }
            .into());
        }

        memory.write(GuestPhysAddr::new(data.address), &self.sector0)?;
        memory.write(GuestPhysAddr::new(status.address), &[VIRTIO_BLK_S_OK])?;
        let written = (VIRTIO_BLK_SECTOR_SIZE + 1) as u32;
        let used_slot = self.last_used_idx % self.queue_size;
        let used_element = checked_add(self.queue_device, 4 + 8 * u64::from(used_slot))?;
        let mut element = [0_u8; 8];
        element[0..4].copy_from_slice(&u32::from(head).to_le_bytes());
        element[4..8].copy_from_slice(&written.to_le_bytes());
        memory.write(GuestPhysAddr::new(used_element), &element)?;
        let next_used = self.last_used_idx.wrapping_add(1);
        memory.write(
            GuestPhysAddr::new(checked_add(self.queue_device, 2)?),
            &next_used.to_le_bytes(),
        )?;

        self.last_avail_idx = avail_idx;
        self.last_used_idx = next_used;
        self.notify_pending = false;
        self.isr_status |= VIRTIO_ISR_QUEUE_INTERRUPT;
        Ok(VirtioBlkQueueCompletion {
            descriptor_id: u32::from(head),
            length: written,
            sector,
        })
    }

    fn read_descriptor(
        &self,
        memory: &GuestMemory,
        index: u16,
    ) -> Result<Descriptor, VirtioBlkProcessError> {
        let address = checked_add(self.queue_desc, 16 * u64::from(index))?;
        let mut bytes = [0_u8; 16];
        memory.read(GuestPhysAddr::new(address), &mut bytes)?;
        Ok(Descriptor {
            address: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            length: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            flags: u16::from_le_bytes(bytes[12..14].try_into().unwrap()),
            next: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
        })
    }

    fn ensure_descriptor_index(&self, index: u16) -> Result<(), VirtioBlkError> {
        if index >= self.queue_size {
            Err(VirtioBlkError::DescriptorIndexOutOfRange {
                index,
                queue_size: self.queue_size,
            })
        } else {
            Ok(())
        }
    }

    fn require_flags(&self, index: u16, actual: u16, expected: u16) -> Result<(), VirtioBlkError> {
        if actual == expected {
            Ok(())
        } else {
            Err(VirtioBlkError::InvalidDescriptorFlags {
                index,
                expected,
                actual,
            })
        }
    }

    fn require_length(&self, index: u16, length: u32, required: u32) -> Result<(), VirtioBlkError> {
        if length >= required {
            Ok(())
        } else {
            Err(VirtioBlkError::DescriptorTooSmall {
                index,
                length,
                required,
            })
        }
    }

    fn device_feature_word(&self) -> u32 {
        match self.device_feature_select {
            1 => (VIRTIO_F_VERSION_1 >> 32) as u32,
            _ => 0,
        }
    }

    fn driver_feature_word(&self) -> u32 {
        match self.driver_feature_select {
            0 => self.driver_features as u32,
            1 => (self.driver_features >> 32) as u32,
            _ => 0,
        }
    }

    fn write_driver_feature_word(&mut self, value: u32) -> Result<(), VirtioBlkError> {
        if self.status & VIRTIO_STATUS_FEATURES_OK != 0 {
            return Err(VirtioBlkError::InvalidDeviceStatus {
                current: self.status,
                requested: self.status,
            });
        }
        match self.driver_feature_select {
            0 => self.driver_features = (self.driver_features & !0xffff_ffff) | u64::from(value),
            1 => {
                self.driver_features =
                    (self.driver_features & 0xffff_ffff) | (u64::from(value) << 32)
            }
            _ if value == 0 => {}
            _ => {
                return Err(VirtioBlkError::InvalidDeviceStatus {
                    current: self.status,
                    requested: self.status,
                })
            }
        }
        Ok(())
    }

    fn write_status(&mut self, requested: u8) -> Result<(), VirtioBlkError> {
        if requested == 0 {
            self.reset();
            return Ok(());
        }
        if requested & !VIRTIO_STATUS_KNOWN != 0 || requested & self.status != self.status {
            return Err(VirtioBlkError::InvalidDeviceStatus {
                current: self.status,
                requested,
            });
        }
        if requested & VIRTIO_STATUS_DRIVER != 0 && requested & VIRTIO_STATUS_ACKNOWLEDGE == 0 {
            return Err(VirtioBlkError::InvalidDeviceStatus {
                current: self.status,
                requested,
            });
        }
        if requested & VIRTIO_STATUS_FEATURES_OK != 0 {
            if requested & (VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER)
                != (VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER)
            {
                return Err(VirtioBlkError::InvalidDeviceStatus {
                    current: self.status,
                    requested,
                });
            }
            if self.driver_features != VIRTIO_F_VERSION_1 {
                self.status = requested & !VIRTIO_STATUS_FEATURES_OK;
                return Ok(());
            }
        }
        if requested & VIRTIO_STATUS_DRIVER_OK != 0
            && (requested & VIRTIO_STATUS_FEATURES_OK == 0 || !self.queue_enabled)
        {
            return Err(VirtioBlkError::InvalidDeviceStatus {
                current: self.status,
                requested,
            });
        }
        self.status = requested;
        Ok(())
    }

    fn write_queue_size(&mut self, size: u16) -> Result<(), VirtioBlkError> {
        self.ensure_selected_queue()?;
        self.ensure_queue_unlocked()?;
        if size == 0 || size > VIRTIO_QUEUE_MAX_SIZE || !size.is_power_of_two() {
            return Err(VirtioBlkError::InvalidQueueSize {
                size,
                maximum: VIRTIO_QUEUE_MAX_SIZE,
            });
        }
        self.queue_size = size;
        Ok(())
    }

    fn write_queue_enable(&mut self, value: u16) -> Result<(), VirtioBlkError> {
        self.ensure_selected_queue()?;
        match value {
            0 => self.queue_enabled = false,
            1 if self.queue_desc != 0 && self.queue_driver != 0 && self.queue_device != 0 => {
                self.queue_enabled = true
            }
            _ => return Err(VirtioBlkError::QueueNotReady),
        }
        Ok(())
    }

    fn selected_queue_size(&self) -> u16 {
        if self.queue_select == VIRTIO_BLK_QUEUE_INDEX {
            self.queue_size
        } else {
            0
        }
    }

    fn ensure_selected_queue(&self) -> Result<(), VirtioBlkError> {
        if self.queue_select == VIRTIO_BLK_QUEUE_INDEX {
            Ok(())
        } else {
            Err(VirtioBlkError::UnsupportedQueue {
                queue: self.queue_select,
            })
        }
    }

    fn ensure_queue_unlocked(&self) -> Result<(), VirtioBlkError> {
        self.ensure_selected_queue()?;
        if self.queue_enabled {
            Err(VirtioBlkError::QueueConfigurationLocked)
        } else {
            Ok(())
        }
    }

    fn ensure_queue_ready(&self) -> Result<(), VirtioBlkError> {
        if self.status & VIRTIO_STATUS_DRIVER_OK == 0
            || self.status & VIRTIO_STATUS_FEATURES_OK == 0
            || self.driver_features != VIRTIO_F_VERSION_1
            || !self.queue_enabled
            || self.queue_desc == 0
            || self.queue_driver == 0
            || self.queue_device == 0
        {
            Err(VirtioBlkError::QueueNotReady)
        } else {
            Ok(())
        }
    }

    fn reset(&mut self) {
        self.device_feature_select = 0;
        self.driver_feature_select = 0;
        self.driver_features = 0;
        self.status = 0;
        self.queue_select = 0;
        self.queue_size = VIRTIO_QUEUE_MAX_SIZE;
        self.queue_enabled = false;
        self.queue_desc = 0;
        self.queue_driver = 0;
        self.queue_device = 0;
        self.notify_pending = false;
        self.last_avail_idx = 0;
        self.last_used_idx = 0;
        self.isr_status = 0;
    }
}

#[derive(Debug, Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

#[must_use]
pub fn deterministic_sector() -> [u8; VIRTIO_BLK_SECTOR_SIZE] {
    let mut bytes = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(17).wrapping_add(3);
    }
    bytes[..16].copy_from_slice(b"BLK-SECTOR-0000!");
    bytes[VIRTIO_BLK_SECTOR_SIZE - 8..].copy_from_slice(b"BLKEND!!");
    bytes
}

fn read_u16(offset: u64, payload: &[u8]) -> Result<u16, VirtioBlkError> {
    let bytes: [u8; 2] =
        payload
            .try_into()
            .map_err(|_| VirtioBlkError::InvalidRegisterPayload {
                offset,
                expected: 2,
                actual: payload.len(),
            })?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(offset: u64, payload: &[u8]) -> Result<u32, VirtioBlkError> {
    let bytes: [u8; 4] =
        payload
            .try_into()
            .map_err(|_| VirtioBlkError::InvalidRegisterPayload {
                offset,
                expected: 4,
                actual: payload.len(),
            })?;
    Ok(u32::from_le_bytes(bytes))
}

const fn replace_low_u32(original: u64, value: u32) -> u64 {
    (original & 0xffff_ffff_0000_0000) | value as u64
}

const fn replace_high_u32(original: u64, value: u32) -> u64 {
    (original & 0x0000_0000_ffff_ffff) | ((value as u64) << 32)
}

fn checked_add(base: u64, offset: u64) -> Result<u64, VirtioBlkError> {
    base.checked_add(offset)
        .ok_or(VirtioBlkError::AddressOverflow { base, offset })
}

fn read_guest_u16(memory: &GuestMemory, address: u64) -> Result<u16, Error> {
    let mut bytes = [0_u8; 2];
    memory.read(GuestPhysAddr::new(address), &mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: u64 = 0x1000_0000;
    const DESC: u64 = 0x18000;
    const AVAIL: u64 = 0x18100;
    const USED: u64 = 0x18200;
    const HEADER: u64 = 0x18300;
    const DATA: u64 = 0x18400;
    const STATUS: u64 = 0x18600;

    #[test]
    fn pci_function_exposes_modern_blk_identity_and_device_config_capability() {
        let function = VirtioBlkPciFunction::new(BAR as u32);
        assert_eq!(
            function.read_dword(0x00),
            (u32::from(VIRTIO_BLK_PCI_DEVICE_ID) << 16) | u32::from(VIRTIO_PCI_VENDOR_ID)
        );
        assert_eq!(function.read_dword(0x10), BAR as u32);
        assert_eq!(function.read_dword(0x64).to_le_bytes(), [0x09, 0x74, 16, 3]);
        assert_eq!(function.read_dword(0x74).to_le_bytes(), [0x09, 0, 16, 4]);
        assert_eq!(function.read_dword(0x7c), VIRTIO_BLK_CONFIG_OFFSET as u32);
        assert_eq!(function.read_dword(0x80), VIRTIO_BLK_CONFIG_LENGTH);
    }

    #[test]
    fn capacity_register_is_one_sector_and_sector_payload_is_stable() {
        let mut device = VirtioBlkDevice::new(BAR);
        assert_eq!(
            device.read(VIRTIO_BLK_CONFIG_OFFSET, 8).unwrap(),
            1_u64.to_le_bytes()
        );
        let sector = deterministic_sector();
        assert_eq!(&sector[..16], b"BLK-SECTOR-0000!");
        assert_eq!(&sector[VIRTIO_BLK_SECTOR_SIZE - 8..], b"BLKEND!!");
    }

    #[test]
    fn three_descriptor_read_request_fills_data_status_and_used_ring() {
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), 0x20_000).unwrap();
        let mut device = VirtioBlkDevice::new(BAR);
        device.driver_features = VIRTIO_F_VERSION_1;
        device.status = VIRTIO_STATUS_ACKNOWLEDGE
            | VIRTIO_STATUS_DRIVER
            | VIRTIO_STATUS_FEATURES_OK
            | VIRTIO_STATUS_DRIVER_OK;
        device.queue_size = 4;
        device.queue_enabled = true;
        device.queue_desc = DESC;
        device.queue_driver = AVAIL;
        device.queue_device = USED;
        device.notify_pending = true;

        write_descriptor(&mut memory, DESC, 0, HEADER, 16, VIRTQ_DESC_F_NEXT, 1);
        write_descriptor(
            &mut memory,
            DESC,
            1,
            DATA,
            VIRTIO_BLK_SECTOR_SIZE as u32,
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
            2,
        );
        write_descriptor(&mut memory, DESC, 2, STATUS, 1, VIRTQ_DESC_F_WRITE, 0);
        let mut header = [0_u8; 16];
        header[0..4].copy_from_slice(&VIRTIO_BLK_T_IN.to_le_bytes());
        header[8..16].copy_from_slice(&0_u64.to_le_bytes());
        memory.write(GuestPhysAddr::new(HEADER), &header).unwrap();
        memory
            .write(GuestPhysAddr::new(AVAIL + 2), &1_u16.to_le_bytes())
            .unwrap();
        memory
            .write(GuestPhysAddr::new(AVAIL + 4), &0_u16.to_le_bytes())
            .unwrap();

        let completion = device.process_notified_queue(&mut memory).unwrap();
        assert_eq!(completion.descriptor_id(), 0);
        assert_eq!(completion.length(), 513);
        assert_eq!(completion.sector(), 0);
        let mut data = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
        memory.read(GuestPhysAddr::new(DATA), &mut data).unwrap();
        assert_eq!(data, deterministic_sector());
        let mut status = [0xff_u8];
        memory
            .read(GuestPhysAddr::new(STATUS), &mut status)
            .unwrap();
        assert_eq!(status, [VIRTIO_BLK_S_OK]);
        assert_eq!(read_guest_u16(&memory, USED + 2).unwrap(), 1);
        assert_eq!(device.isr_status(), VIRTIO_ISR_QUEUE_INTERRUPT);
        assert_eq!(device.read(VIRTIO_ISR_OFFSET, 1).unwrap(), vec![1]);
        assert_eq!(device.read(VIRTIO_ISR_OFFSET, 1).unwrap(), vec![0]);
    }

    fn write_descriptor(
        memory: &mut GuestMemory,
        table: u64,
        index: u16,
        address: u64,
        length: u32,
        flags: u16,
        next: u16,
    ) {
        let mut descriptor = [0_u8; 16];
        descriptor[0..8].copy_from_slice(&address.to_le_bytes());
        descriptor[8..12].copy_from_slice(&length.to_le_bytes());
        descriptor[12..14].copy_from_slice(&flags.to_le_bytes());
        descriptor[14..16].copy_from_slice(&next.to_le_bytes());
        memory
            .write(
                GuestPhysAddr::new(table + 16 * u64::from(index)),
                &descriptor,
            )
            .unwrap();
    }
}
