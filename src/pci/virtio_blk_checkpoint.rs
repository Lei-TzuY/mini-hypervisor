use super::*;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtioBlkCheckpointStateError {
    NotQuiescent,
    MisalignedBar { bar0: u64 },
    UnsupportedDriverFeatures { features: u64 },
    InvalidStatus { status: u8 },
    InvalidQueueSize { size: u16 },
    QueueEnabledWithoutAddresses,
    DriverOkWithoutReadyQueue,
    InvalidIsrStatus { status: u8 },
}

impl fmt::Display for VirtioBlkCheckpointStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotQuiescent => write!(f, "virtio-blk checkpoint state is not quiescent"),
            Self::MisalignedBar { bar0 } => {
                write!(f, "virtio-blk BAR {bar0:#x} is not {:#x}-aligned", VIRTIO_BLK_BAR_SIZE)
            }
            Self::UnsupportedDriverFeatures { features } => write!(
                f,
                "virtio-blk checkpoint contains unsupported driver features {features:#x}"
            ),
            Self::InvalidStatus { status } => {
                write!(f, "virtio-blk checkpoint contains invalid device status {status:#x}")
            }
            Self::InvalidQueueSize { size } => write!(
                f,
                "virtio-blk checkpoint queue size {size} is not a bounded non-zero power of two"
            ),
            Self::QueueEnabledWithoutAddresses => write!(
                f,
                "virtio-blk checkpoint enables the queue without complete descriptor/driver/device addresses"
            ),
            Self::DriverOkWithoutReadyQueue => write!(
                f,
                "virtio-blk checkpoint reports DRIVER_OK without a negotiated ready queue"
            ),
            Self::InvalidIsrStatus { status } => write!(
                f,
                "virtio-blk checkpoint contains unsupported ISR status bits {status:#x}"
            ),
        }
    }
}

impl std::error::Error for VirtioBlkCheckpointStateError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VirtioBlkCheckpointState {
    pub(crate) bar0: u64,
    pub(crate) device_feature_select: u32,
    pub(crate) driver_feature_select: u32,
    pub(crate) driver_features: u64,
    pub(crate) status: u8,
    pub(crate) queue_select: u16,
    pub(crate) queue_size: u16,
    pub(crate) queue_enabled: bool,
    pub(crate) queue_desc: u64,
    pub(crate) queue_driver: u64,
    pub(crate) queue_device: u64,
    pub(crate) last_avail_idx: u16,
    pub(crate) last_used_idx: u16,
    pub(crate) isr_status: u8,
    pub(crate) backing: [u8; VIRTIO_BLK_BACKING_SIZE],
}

impl VirtioBlkCheckpointState {
    pub(crate) fn capture(device: &VirtioBlkDevice) -> Result<Self, VirtioBlkCheckpointStateError> {
        if !device.checkpoint_quiescent() {
            return Err(VirtioBlkCheckpointStateError::NotQuiescent);
        }
        let state = Self {
            bar0: device.bar0,
            device_feature_select: device.device_feature_select,
            driver_feature_select: device.driver_feature_select,
            driver_features: device.driver_features,
            status: device.status,
            queue_select: device.queue_select,
            queue_size: device.queue_size,
            queue_enabled: device.queue_enabled,
            queue_desc: device.queue_desc,
            queue_driver: device.queue_driver,
            queue_device: device.queue_device,
            last_avail_idx: device.last_avail_idx,
            last_used_idx: device.last_used_idx,
            isr_status: device.isr_status,
            backing: device.backing,
        };
        state.validate()?;
        Ok(state)
    }

    pub(crate) fn materialize(&self) -> Result<VirtioBlkDevice, VirtioBlkCheckpointStateError> {
        self.validate()?;
        Ok(VirtioBlkDevice {
            bar0: self.bar0,
            device_feature_select: self.device_feature_select,
            driver_feature_select: self.driver_feature_select,
            driver_features: self.driver_features,
            status: self.status,
            queue_select: self.queue_select,
            queue_size: self.queue_size,
            queue_enabled: self.queue_enabled,
            queue_desc: self.queue_desc,
            queue_driver: self.queue_driver,
            queue_device: self.queue_device,
            notify_pending: false,
            last_avail_idx: self.last_avail_idx,
            last_used_idx: self.last_used_idx,
            isr_status: self.isr_status,
            backing: self.backing,
        })
    }

    pub(crate) fn validate(&self) -> Result<(), VirtioBlkCheckpointStateError> {
        if self.bar0 % u64::from(VIRTIO_BLK_BAR_SIZE) != 0 {
            return Err(VirtioBlkCheckpointStateError::MisalignedBar { bar0: self.bar0 });
        }
        if self.driver_features & !VIRTIO_BLK_SUPPORTED_FEATURES != 0 {
            return Err(VirtioBlkCheckpointStateError::UnsupportedDriverFeatures {
                features: self.driver_features,
            });
        }
        if self.status & !VIRTIO_STATUS_KNOWN != 0
            || (self.status & VIRTIO_STATUS_DRIVER != 0
                && self.status & VIRTIO_STATUS_ACKNOWLEDGE == 0)
            || (self.status & VIRTIO_STATUS_FEATURES_OK != 0
                && (self.status & (VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER)
                    != (VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER)
                    || self.driver_features & VIRTIO_F_VERSION_1 == 0))
        {
            return Err(VirtioBlkCheckpointStateError::InvalidStatus {
                status: self.status,
            });
        }
        if self.queue_size == 0
            || self.queue_size > VIRTIO_QUEUE_MAX_SIZE
            || !self.queue_size.is_power_of_two()
        {
            return Err(VirtioBlkCheckpointStateError::InvalidQueueSize {
                size: self.queue_size,
            });
        }
        if self.queue_enabled
            && (self.queue_desc == 0 || self.queue_driver == 0 || self.queue_device == 0)
        {
            return Err(VirtioBlkCheckpointStateError::QueueEnabledWithoutAddresses);
        }
        if self.status & VIRTIO_STATUS_DRIVER_OK != 0
            && (self.status & VIRTIO_STATUS_FEATURES_OK == 0
                || !self.queue_enabled
                || self.driver_features & VIRTIO_F_VERSION_1 == 0)
        {
            return Err(VirtioBlkCheckpointStateError::DriverOkWithoutReadyQueue);
        }
        if self.isr_status & !VIRTIO_ISR_QUEUE_INTERRUPT != 0 {
            return Err(VirtioBlkCheckpointStateError::InvalidIsrStatus {
                status: self.isr_status,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: u64 = 0x1000_0000;

    fn ready_device() -> VirtioBlkDevice {
        let mut device = VirtioBlkDevice::new(BAR);
        device.driver_features = VIRTIO_F_VERSION_1;
        device.status = VIRTIO_STATUS_ACKNOWLEDGE
            | VIRTIO_STATUS_DRIVER
            | VIRTIO_STATUS_FEATURES_OK
            | VIRTIO_STATUS_DRIVER_OK;
        device.queue_size = 4;
        device.queue_enabled = true;
        device.queue_desc = 0x18_000;
        device.queue_driver = 0x18_100;
        device.queue_device = 0x18_200;
        device.last_avail_idx = 7;
        device.last_used_idx = 7;
        device.isr_status = VIRTIO_ISR_QUEUE_INTERRUPT;
        device.backing[700] = 0x5a;
        device
    }

    #[test]
    fn semantic_checkpoint_round_trip_preserves_exact_quiescent_device_state() {
        let device = ready_device();
        let state = VirtioBlkCheckpointState::capture(&device).unwrap();
        let restored = state.materialize().unwrap();
        assert_eq!(restored, device);
        assert!(restored.checkpoint_quiescent());
        assert_eq!(restored.backing_bytes()[700], 0x5a);
    }

    #[test]
    fn pending_notification_is_not_checkpointable() {
        let mut device = ready_device();
        device.notify_pending = true;
        assert_eq!(
            VirtioBlkCheckpointState::capture(&device),
            Err(VirtioBlkCheckpointStateError::NotQuiescent)
        );
    }

    #[test]
    fn invalid_semantic_state_fails_before_materialization() {
        let device = ready_device();
        let mut state = VirtioBlkCheckpointState::capture(&device).unwrap();

        state.queue_size = 3;
        assert!(matches!(
            state.materialize(),
            Err(VirtioBlkCheckpointStateError::InvalidQueueSize { size: 3 })
        ));

        let mut state = VirtioBlkCheckpointState::capture(&device).unwrap();
        state.status |= 0x80;
        assert!(matches!(
            state.materialize(),
            Err(VirtioBlkCheckpointStateError::InvalidStatus { .. })
        ));

        let mut state = VirtioBlkCheckpointState::capture(&device).unwrap();
        state.queue_desc = 0;
        assert_eq!(
            state.materialize(),
            Err(VirtioBlkCheckpointStateError::QueueEnabledWithoutAddresses)
        );
    }
}
