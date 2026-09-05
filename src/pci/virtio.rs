use crate::error::Error;
use crate::memory::{GuestMemory, GuestPhysAddr};
use std::fmt;

pub const VIRTIO_PCI_VENDOR_ID: u16 = 0x1af4;
pub const VIRTIO_RNG_DEVICE_TYPE: u16 = 4;
pub const VIRTIO_RNG_PCI_DEVICE_ID: u16 = 0x1040 + VIRTIO_RNG_DEVICE_TYPE;
pub const VIRTIO_RNG_PCI_REVISION: u8 = 1;
pub const VIRTIO_RNG_PCI_CLASS_CODE: u8 = 0xff;
pub const VIRTIO_PCI_CAP_VENDOR_ID: u8 = 0x09;
pub const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
pub const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
pub const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
pub const VIRTIO_F_VERSION_1: u64 = 1_u64 << 32;

pub const VIRTIO_RNG_BAR_SIZE: u32 = 0x1000;
pub const VIRTIO_COMMON_OFFSET: u64 = 0x000;
pub const VIRTIO_COMMON_LENGTH: u32 = 0x38;
pub const VIRTIO_NOTIFY_OFFSET: u64 = 0x100;
pub const VIRTIO_NOTIFY_LENGTH: u32 = 2;
pub const VIRTIO_NOTIFY_OFF_MULTIPLIER: u32 = 2;
pub const VIRTIO_ISR_OFFSET: u64 = 0x200;
pub const VIRTIO_ISR_LENGTH: u32 = 1;
pub const VIRTIO_QUEUE_MAX_SIZE: u16 = 8;
pub const VIRTIO_RNG_QUEUE_INDEX: u16 = 0;
pub const VIRTQ_DESC_F_WRITE: u16 = 2;
pub const VIRTIO_RNG_TEST_PAYLOAD: &[u8; 8] = b"RNGDATA!";

pub const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
pub const VIRTIO_STATUS_DRIVER: u8 = 2;
pub const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
pub const VIRTIO_STATUS_FEATURES_OK: u8 = 8;
pub const VIRTIO_STATUS_FAILED: u8 = 0x80;
const VIRTIO_STATUS_KNOWN: u8 = VIRTIO_STATUS_ACKNOWLEDGE
    | VIRTIO_STATUS_DRIVER
    | VIRTIO_STATUS_DRIVER_OK
    | VIRTIO_STATUS_FEATURES_OK
    | VIRTIO_STATUS_FAILED;

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
const COMMON_QUEUE_DESC_HIGH: u64 = COMMON_QUEUE_DESC + 4;
const COMMON_QUEUE_DRIVER: u64 = 0x28;
const COMMON_QUEUE_DRIVER_HIGH: u64 = COMMON_QUEUE_DRIVER + 4;
const COMMON_QUEUE_DEVICE: u64 = 0x30;
const COMMON_QUEUE_DEVICE_HIGH: u64 = COMMON_QUEUE_DEVICE + 4;

const PCI_STATUS_CAPABILITIES_LIST: u16 = 1 << 4;
const VIRTIO_CAP_COMMON: u8 = 0x40;
const VIRTIO_CAP_NOTIFY: u8 = 0x50;
const VIRTIO_CAP_ISR: u8 = 0x64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtioRngError {
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
    UnsupportedDescriptorFlags {
        flags: u16,
    },
    DescriptorTooSmall {
        length: u32,
        required: u32,
    },
    AddressOverflow {
        base: u64,
        offset: u64,
    },
}

impl fmt::Display for VirtioRngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedRegisterAccess {
                offset,
                length,
                write,
            } => write!(
                f,
                "unsupported virtio-rng {} access at BAR offset {offset:#x} with length {length}",
                if *write { "write" } else { "read" }
            ),
            Self::InvalidRegisterPayload {
                offset,
                expected,
                actual,
            } => write!(
                f,
                "invalid virtio-rng write payload at BAR offset {offset:#x}: expected {expected} bytes, got {actual}"
            ),
            Self::InvalidDeviceStatus { current, requested } => write!(
                f,
                "invalid virtio device-status transition {current:#x} -> {requested:#x}"
            ),
            Self::UnsupportedQueue { queue } => {
                write!(f, "virtio-rng exposes only requestq 0, not queue {queue}")
            }
            Self::InvalidQueueSize { size, maximum } => write!(
                f,
                "invalid virtio-rng queue size {size}; maximum power-of-two size is {maximum}"
            ),
            Self::QueueConfigurationLocked => {
                write!(f, "virtio-rng queue configuration is locked while queue 0 is enabled")
            }
            Self::QueueNotReady => write!(f, "virtio-rng requestq is not fully negotiated and enabled"),
            Self::UnexpectedQueueNotification { queue } => {
                write!(f, "virtio-rng received notification for unsupported queue {queue}")
            }
            Self::UnexpectedAvailIndex { expected, actual } => write!(
                f,
                "virtio-rng expected avail.idx {expected}, got {actual}"
            ),
            Self::UnexpectedUsedIndex { expected, actual } => write!(
                f,
                "virtio-rng expected used.idx {expected}, got {actual}"
            ),
            Self::DescriptorIndexOutOfRange { index, queue_size } => write!(
                f,
                "virtio-rng descriptor index {index} is outside queue size {queue_size}"
            ),
            Self::UnsupportedDescriptorFlags { flags } => write!(
                f,
                "virtio-rng first slice requires one device-writable direct descriptor; flags={flags:#x}"
            ),
            Self::DescriptorTooSmall { length, required } => write!(
                f,
                "virtio-rng writable descriptor length {length} is smaller than required {required}"
            ),
            Self::AddressOverflow { base, offset } => write!(
                f,
                "virtio-rng guest address arithmetic overflows: base={base:#x}, offset={offset:#x}"
            ),
        }
    }
}

impl std::error::Error for VirtioRngError {}

#[derive(Debug)]
pub enum VirtioRngProcessError {
    Device(VirtioRngError),
    Memory(Error),
}

impl fmt::Display for VirtioRngProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(error) => error.fmt(f),
            Self::Memory(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for VirtioRngProcessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Device(error) => Some(error),
            Self::Memory(error) => Some(error),
        }
    }
}

impl From<VirtioRngError> for VirtioRngProcessError {
    fn from(value: VirtioRngError) -> Self {
        Self::Device(value)
    }
}

impl From<Error> for VirtioRngProcessError {
    fn from(value: Error) -> Self {
        Self::Memory(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioRngQueueCompletion {
    descriptor_id: u32,
    length: u32,
}

impl VirtioRngQueueCompletion {
    #[must_use]
    pub const fn descriptor_id(self) -> u32 {
        self.descriptor_id
    }

    #[must_use]
    pub const fn length(self) -> u32 {
        self.length
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioRngPciFunction {
    bar0: u32,
}

impl VirtioRngPciFunction {
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
            0x00 => (u32::from(VIRTIO_RNG_PCI_DEVICE_ID) << 16) | u32::from(VIRTIO_PCI_VENDOR_ID),
            0x04 => u32::from(PCI_STATUS_CAPABILITIES_LIST) << 16,
            0x08 => (u32::from(VIRTIO_RNG_PCI_CLASS_CODE) << 24) | u32::from(VIRTIO_RNG_PCI_REVISION),
            0x0c => 0,
            0x10 => self.bar0,
            0x2c => (0x0044_u32 << 16) | u32::from(VIRTIO_PCI_VENDOR_ID),
            0x34 => u32::from(VIRTIO_CAP_COMMON),
            0x40 => capability_header(VIRTIO_CAP_NOTIFY, 16, VIRTIO_PCI_CAP_COMMON_CFG),
            0x44 => 0,
            0x48 => VIRTIO_COMMON_OFFSET as u32,
            0x4c => VIRTIO_COMMON_LENGTH,
            0x50 => capability_header(VIRTIO_CAP_ISR, 20, VIRTIO_PCI_CAP_NOTIFY_CFG),
            0x54 => 0,
            0x58 => VIRTIO_NOTIFY_OFFSET as u32,
            0x5c => VIRTIO_NOTIFY_LENGTH,
            0x60 => VIRTIO_NOTIFY_OFF_MULTIPLIER,
            0x64 => capability_header(0, 16, VIRTIO_PCI_CAP_ISR_CFG),
            0x68 => 0,
            0x6c => VIRTIO_ISR_OFFSET as u32,
            0x70 => VIRTIO_ISR_LENGTH,
            _ => 0,
        }
    }
}

const fn capability_header(next: u8, length: u8, cfg_type: u8) -> u32 {
    u32::from_le_bytes([VIRTIO_PCI_CAP_VENDOR_ID, next, length, cfg_type])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioRngEvent {
    QueueNotified { queue: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioRngDevice {
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
}

impl VirtioRngDevice {
    #[must_use]
    pub const fn new(bar0: u64) -> Self {
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

    pub fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>, VirtioRngError> {
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
            (COMMON_QUEUE_DRIVER_HIGH, 4) => ((self.queue_driver >> 32) as u32).to_le_bytes().to_vec(),
            (COMMON_QUEUE_DEVICE, 4) => (self.queue_device as u32).to_le_bytes().to_vec(),
            (COMMON_QUEUE_DEVICE_HIGH, 4) => ((self.queue_device >> 32) as u32).to_le_bytes().to_vec(),
            (VIRTIO_ISR_OFFSET, 1) => vec![0],
            _ => {
                return Err(VirtioRngError::UnsupportedRegisterAccess {
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
    ) -> Result<Option<VirtioRngEvent>, VirtioRngError> {
        match (offset, payload.len()) {
            (COMMON_DEVICE_FEATURE_SELECT, 4) => {
                self.device_feature_select = read_u32(offset, payload)?;
            }
            (COMMON_DRIVER_FEATURE_SELECT, 4) => {
                self.driver_feature_select = read_u32(offset, payload)?;
            }
            (COMMON_DRIVER_FEATURE, 4) => {
                let value = read_u32(offset, payload)?;
                self.write_driver_feature_word(value)?;
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
                if queue != VIRTIO_RNG_QUEUE_INDEX {
                    return Err(VirtioRngError::UnexpectedQueueNotification { queue });
                }
                self.ensure_queue_ready()?;
                self.notify_pending = true;
                return Ok(Some(VirtioRngEvent::QueueNotified { queue }));
            }
            _ => {
                return Err(VirtioRngError::UnsupportedRegisterAccess {
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
    ) -> Result<VirtioRngQueueCompletion, VirtioRngProcessError> {
        self.ensure_queue_ready()?;
        if !self.notify_pending {
            return Err(VirtioRngError::QueueNotReady.into());
        }

        let avail_idx = read_guest_u16(memory, checked_add(self.queue_driver, 2)?)?;
        let expected_avail = self.last_avail_idx.wrapping_add(1);
        if avail_idx != expected_avail {
            return Err(VirtioRngError::UnexpectedAvailIndex {
                expected: expected_avail,
                actual: avail_idx,
            }
            .into());
        }

        let slot = self.last_avail_idx % self.queue_size;
        let ring_offset = 4_u64 + 2_u64 * u64::from(slot);
        let head = read_guest_u16(memory, checked_add(self.queue_driver, ring_offset)?)?;
        if head >= self.queue_size {
            return Err(VirtioRngError::DescriptorIndexOutOfRange {
                index: head,
                queue_size: self.queue_size,
            }
            .into());
        }

        let descriptor_address = checked_add(self.queue_desc, 16_u64 * u64::from(head))?;
        let mut descriptor = [0_u8; 16];
        memory.read(GuestPhysAddr::new(descriptor_address), &mut descriptor)?;
        let buffer_address = u64::from_le_bytes(descriptor[0..8].try_into().unwrap());
        let buffer_length = u32::from_le_bytes(descriptor[8..12].try_into().unwrap());
        let flags = u16::from_le_bytes(descriptor[12..14].try_into().unwrap());
        if flags != VIRTQ_DESC_F_WRITE {
            return Err(VirtioRngError::UnsupportedDescriptorFlags { flags }.into());
        }
        let required = u32::try_from(VIRTIO_RNG_TEST_PAYLOAD.len()).unwrap();
        if buffer_length < required {
            return Err(VirtioRngError::DescriptorTooSmall {
                length: buffer_length,
                required,
            }
            .into());
        }

        let used_idx = read_guest_u16(memory, checked_add(self.queue_device, 2)?)?;
        if used_idx != self.last_used_idx {
            return Err(VirtioRngError::UnexpectedUsedIndex {
                expected: self.last_used_idx,
                actual: used_idx,
            }
            .into());
        }

        memory.write(
            GuestPhysAddr::new(buffer_address),
            VIRTIO_RNG_TEST_PAYLOAD,
        )?;
        let used_slot = self.last_used_idx % self.queue_size;
        let used_element = checked_add(self.queue_device, 4_u64 + 8_u64 * u64::from(used_slot))?;
        let mut element = [0_u8; 8];
        element[0..4].copy_from_slice(&u32::from(head).to_le_bytes());
        element[4..8].copy_from_slice(&required.to_le_bytes());
        memory.write(GuestPhysAddr::new(used_element), &element)?;
        let next_used = self.last_used_idx.wrapping_add(1);
        memory.write(
            GuestPhysAddr::new(checked_add(self.queue_device, 2)?),
            &next_used.to_le_bytes(),
        )?;

        self.last_avail_idx = avail_idx;
        self.last_used_idx = next_used;
        self.notify_pending = false;
        Ok(VirtioRngQueueCompletion {
            descriptor_id: u32::from(head),
            length: required,
        })
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

    fn write_driver_feature_word(&mut self, value: u32) -> Result<(), VirtioRngError> {
        if self.status & VIRTIO_STATUS_FEATURES_OK != 0 {
            return Err(VirtioRngError::InvalidDeviceStatus {
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
                return Err(VirtioRngError::InvalidDeviceStatus {
                    current: self.status,
                    requested: self.status,
                })
            }
        }
        Ok(())
    }

    fn write_status(&mut self, requested: u8) -> Result<(), VirtioRngError> {
        if requested == 0 {
            self.reset();
            return Ok(());
        }
        if requested & !VIRTIO_STATUS_KNOWN != 0 || requested & self.status != self.status {
            return Err(VirtioRngError::InvalidDeviceStatus {
                current: self.status,
                requested,
            });
        }
        if requested & VIRTIO_STATUS_DRIVER != 0 && requested & VIRTIO_STATUS_ACKNOWLEDGE == 0 {
            return Err(VirtioRngError::InvalidDeviceStatus {
                current: self.status,
                requested,
            });
        }
        if requested & VIRTIO_STATUS_FEATURES_OK != 0 {
            if requested & (VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER)
                != (VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER)
            {
                return Err(VirtioRngError::InvalidDeviceStatus {
                    current: self.status,
                    requested,
                });
            }
            if self.driver_features != VIRTIO_F_VERSION_1 {
                self.status = requested & !VIRTIO_STATUS_FEATURES_OK;
                return Ok(());
            }
        }
        if requested & VIRTIO_STATUS_DRIVER_OK != 0 {
            if requested & VIRTIO_STATUS_FEATURES_OK == 0 || !self.queue_enabled {
                return Err(VirtioRngError::InvalidDeviceStatus {
                    current: self.status,
                    requested,
                });
            }
        }
        self.status = requested;
        Ok(())
    }

    fn write_queue_size(&mut self, size: u16) -> Result<(), VirtioRngError> {
        self.ensure_selected_queue()?;
        self.ensure_queue_unlocked()?;
        if size == 0 || size > VIRTIO_QUEUE_MAX_SIZE || !size.is_power_of_two() {
            return Err(VirtioRngError::InvalidQueueSize {
                size,
                maximum: VIRTIO_QUEUE_MAX_SIZE,
            });
        }
        self.queue_size = size;
        Ok(())
    }

    fn write_queue_enable(&mut self, value: u16) -> Result<(), VirtioRngError> {
        self.ensure_selected_queue()?;
        match value {
            0 => self.queue_enabled = false,
            1 => {
                if self.queue_desc == 0 || self.queue_driver == 0 || self.queue_device == 0 {
                    return Err(VirtioRngError::QueueNotReady);
                }
                self.queue_enabled = true;
            }
            _ => return Err(VirtioRngError::QueueNotReady),
        }
        Ok(())
    }

    fn selected_queue_size(&self) -> u16 {
        if self.queue_select == VIRTIO_RNG_QUEUE_INDEX {
            self.queue_size
        } else {
            0
        }
    }

    fn ensure_selected_queue(&self) -> Result<(), VirtioRngError> {
        if self.queue_select != VIRTIO_RNG_QUEUE_INDEX {
            return Err(VirtioRngError::UnsupportedQueue {
                queue: self.queue_select,
            });
        }
        Ok(())
    }

    fn ensure_queue_unlocked(&self) -> Result<(), VirtioRngError> {
        self.ensure_selected_queue()?;
        if self.queue_enabled {
            return Err(VirtioRngError::QueueConfigurationLocked);
        }
        Ok(())
    }

    fn ensure_queue_ready(&self) -> Result<(), VirtioRngError> {
        if self.status & VIRTIO_STATUS_DRIVER_OK == 0
            || self.status & VIRTIO_STATUS_FEATURES_OK == 0
            || self.driver_features != VIRTIO_F_VERSION_1
            || !self.queue_enabled
            || self.queue_desc == 0
            || self.queue_driver == 0
            || self.queue_device == 0
        {
            return Err(VirtioRngError::QueueNotReady);
        }
        Ok(())
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
    }
}

fn read_u16(offset: u64, payload: &[u8]) -> Result<u16, VirtioRngError> {
    let bytes: [u8; 2] = payload.try_into().map_err(|_| VirtioRngError::InvalidRegisterPayload {
        offset,
        expected: 2,
        actual: payload.len(),
    })?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(offset: u64, payload: &[u8]) -> Result<u32, VirtioRngError> {
    let bytes: [u8; 4] = payload.try_into().map_err(|_| VirtioRngError::InvalidRegisterPayload {
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

fn checked_add(base: u64, offset: u64) -> Result<u64, VirtioRngError> {
    base.checked_add(offset)
        .ok_or(VirtioRngError::AddressOverflow { base, offset })
}

fn read_guest_u16(memory: &GuestMemory, address: u64) -> Result<u16, VirtioRngProcessError> {
    let mut bytes = [0_u8; 2];
    memory.read(GuestPhysAddr::new(address), &mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR0: u64 = 0x1000_0000;
    const DESC: u64 = 0x8000;
    const AVAIL: u64 = 0x9000;
    const USED: u64 = 0xa000;
    const BUFFER: u64 = 0xb000;

    fn write_u16(device: &mut VirtioRngDevice, offset: u64, value: u16) {
        device.write(offset, &value.to_le_bytes()).unwrap();
    }

    fn write_u32(device: &mut VirtioRngDevice, offset: u64, value: u32) {
        device.write(offset, &value.to_le_bytes()).unwrap();
    }

    fn negotiate_and_enable(device: &mut VirtioRngDevice) {
        device.write(COMMON_DEVICE_STATUS, &[VIRTIO_STATUS_ACKNOWLEDGE]).unwrap();
        device
            .write(
                COMMON_DEVICE_STATUS,
                &[VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER],
            )
            .unwrap();
        write_u32(device, COMMON_DRIVER_FEATURE_SELECT, 1);
        write_u32(device, COMMON_DRIVER_FEATURE, 1);
        device
            .write(
                COMMON_DEVICE_STATUS,
                &[VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK],
            )
            .unwrap();
        assert_eq!(
            device.status() & VIRTIO_STATUS_FEATURES_OK,
            VIRTIO_STATUS_FEATURES_OK
        );
        write_u16(device, COMMON_QUEUE_SELECT, 0);
        write_u16(device, COMMON_QUEUE_SIZE, 8);
        write_u32(device, COMMON_QUEUE_DESC, DESC as u32);
        write_u32(device, COMMON_QUEUE_DRIVER, AVAIL as u32);
        write_u32(device, COMMON_QUEUE_DEVICE, USED as u32);
        write_u16(device, COMMON_QUEUE_ENABLE, 1);
        device
            .write(
                COMMON_DEVICE_STATUS,
                &[VIRTIO_STATUS_ACKNOWLEDGE
                    | VIRTIO_STATUS_DRIVER
                    | VIRTIO_STATUS_FEATURES_OK
                    | VIRTIO_STATUS_DRIVER_OK],
            )
            .unwrap();
    }

    #[test]
    fn pci_identity_and_capability_chain_are_modern_virtio_rng() {
        let function = VirtioRngPciFunction::new(BAR0 as u32);
        assert_eq!(
            function.read_dword(0x00),
            (u32::from(VIRTIO_RNG_PCI_DEVICE_ID) << 16) | u32::from(VIRTIO_PCI_VENDOR_ID)
        );
        assert_eq!(function.read_dword(0x10), BAR0 as u32);
        assert_eq!(function.read_dword(0x34) & 0xff, u32::from(VIRTIO_CAP_COMMON));
        assert_eq!(function.read_dword(0x40).to_le_bytes(), [0x09, 0x50, 16, 1]);
        assert_eq!(function.read_dword(0x50).to_le_bytes(), [0x09, 0x64, 20, 2]);
        assert_eq!(function.read_dword(0x64).to_le_bytes(), [0x09, 0x00, 16, 3]);
    }

    #[test]
    fn version_1_feature_and_status_handshake_is_required() {
        let mut device = VirtioRngDevice::new(BAR0);
        write_u32(&mut device, COMMON_DEVICE_FEATURE_SELECT, 1);
        assert_eq!(
            u32::from_le_bytes(device.read(COMMON_DEVICE_FEATURE, 4).unwrap().try_into().unwrap()),
            1
        );

        device.write(COMMON_DEVICE_STATUS, &[VIRTIO_STATUS_ACKNOWLEDGE]).unwrap();
        device
            .write(
                COMMON_DEVICE_STATUS,
                &[VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER],
            )
            .unwrap();
        device
            .write(
                COMMON_DEVICE_STATUS,
                &[VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK],
            )
            .unwrap();
        assert_eq!(device.status() & VIRTIO_STATUS_FEATURES_OK, 0);
    }

    #[test]
    fn one_writable_descriptor_is_filled_and_returned_on_used_ring() {
        let mut device = VirtioRngDevice::new(BAR0);
        negotiate_and_enable(&mut device);
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), 0x20_000).unwrap();

        let mut descriptor = [0_u8; 16];
        descriptor[0..8].copy_from_slice(&BUFFER.to_le_bytes());
        descriptor[8..12].copy_from_slice(&8_u32.to_le_bytes());
        descriptor[12..14].copy_from_slice(&VIRTQ_DESC_F_WRITE.to_le_bytes());
        memory.write(GuestPhysAddr::new(DESC), &descriptor).unwrap();
        memory
            .write(GuestPhysAddr::new(AVAIL + 2), &1_u16.to_le_bytes())
            .unwrap();
        memory
            .write(GuestPhysAddr::new(AVAIL + 4), &0_u16.to_le_bytes())
            .unwrap();

        assert_eq!(
            device
                .write(VIRTIO_NOTIFY_OFFSET, &0_u16.to_le_bytes())
                .unwrap(),
            Some(VirtioRngEvent::QueueNotified { queue: 0 })
        );
        let completion = device.process_notified_queue(&mut memory).unwrap();
        assert_eq!(completion.descriptor_id(), 0);
        assert_eq!(completion.length(), 8);

        let mut payload = [0_u8; 8];
        memory.read(GuestPhysAddr::new(BUFFER), &mut payload).unwrap();
        assert_eq!(&payload, VIRTIO_RNG_TEST_PAYLOAD);
        let mut used = [0_u8; 10];
        memory.read(GuestPhysAddr::new(USED + 2), &mut used).unwrap();
        assert_eq!(u16::from_le_bytes(used[0..2].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(used[2..6].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(used[6..10].try_into().unwrap()), 8);
    }

    #[test]
    fn rejects_readable_or_chained_descriptor_without_mutating_used_index() {
        let mut device = VirtioRngDevice::new(BAR0);
        negotiate_and_enable(&mut device);
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), 0x20_000).unwrap();
        let mut descriptor = [0_u8; 16];
        descriptor[0..8].copy_from_slice(&BUFFER.to_le_bytes());
        descriptor[8..12].copy_from_slice(&8_u32.to_le_bytes());
        descriptor[12..14].copy_from_slice(&0_u16.to_le_bytes());
        memory.write(GuestPhysAddr::new(DESC), &descriptor).unwrap();
        memory
            .write(GuestPhysAddr::new(AVAIL + 2), &1_u16.to_le_bytes())
            .unwrap();
        memory
            .write(GuestPhysAddr::new(AVAIL + 4), &0_u16.to_le_bytes())
            .unwrap();
        device
            .write(VIRTIO_NOTIFY_OFFSET, &0_u16.to_le_bytes())
            .unwrap();

        assert!(matches!(
            device.process_notified_queue(&mut memory),
            Err(VirtioRngProcessError::Device(
                VirtioRngError::UnsupportedDescriptorFlags { flags: 0 }
            ))
        ));
        let mut used_idx = [0_u8; 2];
        memory
            .read(GuestPhysAddr::new(USED + 2), &mut used_idx)
            .unwrap();
        assert_eq!(u16::from_le_bytes(used_idx), 0);
    }
}
