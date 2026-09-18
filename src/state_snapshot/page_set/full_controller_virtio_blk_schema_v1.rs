use crate::portio::pci::virtio_blk::{
    VirtioBlkCheckpointState, VirtioBlkCheckpointStateError, VIRTIO_BLK_BACKING_SIZE,
    VIRTIO_BLK_CAPACITY_SECTORS, VIRTIO_BLK_SECTOR_SIZE,
};

pub const VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_MAGIC: [u8; 8] = *b"MHVFCVB\0";
pub const VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_VERSION: u16 = 1;
pub const VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_ARCH_X86_64: u16 = 1;

const FULL_CONTROLLER_VIRTIO_BLK_HEADER_LEN: usize = 48;
const VIRTIO_BLK_STATE_HEADER_LEN: usize = 80;
const VIRTIO_BLK_STATE_LEN: usize = VIRTIO_BLK_STATE_HEADER_LEN + VIRTIO_BLK_BACKING_SIZE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionedFullControllerVirtioBlkCheckpointError {
    InvalidMagic,
    UnsupportedVersion(u16),
    UnsupportedArchitecture(u16),
    InvalidHeaderLength(u32),
    InvalidTotalLength { declared: u64, actual: usize },
    InvalidControllerLength(u64),
    InvalidDeviceLength(u64),
    NonZeroFlags(u32),
    NonZeroReserved(u32),
    LengthOverflow,
    InvalidQueueEnabled(u8),
    InvalidSectorSize(u32),
    InvalidCapacity(u64),
    InvalidBackingLength(u32),
    NonZeroDeviceReserved(u32),
    BarMismatch { checkpoint: u64, device: u64 },
    Controller(VersionedFullControllerCheckpointError),
    Device(VirtioBlkCheckpointStateError),
}

impl std::fmt::Display for VersionedFullControllerVirtioBlkCheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "full-controller virtio-blk checkpoint magic does not match MHVFCVB"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported full-controller virtio-blk checkpoint version {version}"),
            Self::UnsupportedArchitecture(architecture) => write!(f, "unsupported full-controller virtio-blk architecture identifier {architecture}"),
            Self::InvalidHeaderLength(length) => write!(f, "full-controller virtio-blk header length {length} is not {FULL_CONTROLLER_VIRTIO_BLK_HEADER_LEN}"),
            Self::InvalidTotalLength { declared, actual } => write!(f, "full-controller virtio-blk checkpoint declares total length {declared}, actual byte length is {actual}"),
            Self::InvalidControllerLength(length) => write!(f, "full-controller virtio-blk nested controller length {length} is invalid"),
            Self::InvalidDeviceLength(length) => write!(f, "full-controller virtio-blk device payload length {length} is not {VIRTIO_BLK_STATE_LEN}"),
            Self::NonZeroFlags(flags) => write!(f, "full-controller virtio-blk v1 flags must be zero, got {flags:#x}"),
            Self::NonZeroReserved(value) => write!(f, "full-controller virtio-blk reserved field must be zero, got {value:#x}"),
            Self::LengthOverflow => write!(f, "full-controller virtio-blk checkpoint length arithmetic overflowed"),
            Self::InvalidQueueEnabled(value) => write!(f, "virtio-blk queue-enabled field must be 0 or 1, got {value}"),
            Self::InvalidSectorSize(size) => write!(f, "virtio-blk checkpoint sector size {size} does not match {VIRTIO_BLK_SECTOR_SIZE}"),
            Self::InvalidCapacity(capacity) => write!(f, "virtio-blk checkpoint capacity {capacity} does not match {VIRTIO_BLK_CAPACITY_SECTORS} sectors"),
            Self::InvalidBackingLength(length) => write!(f, "virtio-blk checkpoint backing length {length} does not match {VIRTIO_BLK_BACKING_SIZE}"),
            Self::NonZeroDeviceReserved(value) => write!(f, "virtio-blk checkpoint reserved field must be zero, got {value:#x}"),
            Self::BarMismatch { checkpoint, device } => write!(f, "virtio-blk checkpoint BAR {checkpoint:#x} does not match device BAR {device:#x}"),
            Self::Controller(error) => write!(f, "nested full-controller checkpoint is invalid: {error}"),
            Self::Device(error) => write!(f, "virtio-blk checkpoint state is invalid: {error}"),
        }
    }
}

impl std::error::Error for VersionedFullControllerVirtioBlkCheckpointError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Controller(error) => Some(error),
            Self::Device(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VersionedFullControllerCheckpointError>
    for VersionedFullControllerVirtioBlkCheckpointError
{
    fn from(error: VersionedFullControllerCheckpointError) -> Self {
        Self::Controller(error)
    }
}

impl From<VirtioBlkCheckpointStateError> for VersionedFullControllerVirtioBlkCheckpointError {
    fn from(error: VirtioBlkCheckpointStateError) -> Self {
        Self::Device(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedFullControllerVirtioBlkCheckpointV1 {
    controller: VersionedFullControllerCheckpointV1,
    device: VirtioBlkCheckpointState,
}

impl VersionedFullControllerVirtioBlkCheckpointV1 {
    pub fn from_checkpoint(
        checkpoint: &BoundedFullControllerVirtioBlkCheckpoint,
    ) -> Result<Self, VersionedFullControllerVirtioBlkCheckpointError> {
        let controller = VersionedFullControllerCheckpointV1::from_checkpoint(checkpoint.controller())?;
        let device = VirtioBlkCheckpointState::capture(checkpoint.device())?;
        if checkpoint.bar0() != device.bar0 {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::BarMismatch {
                checkpoint: checkpoint.bar0(),
                device: device.bar0,
            });
        }
        Ok(Self { controller, device })
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_VERSION
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.controller.page_count()
    }

    #[must_use]
    pub fn msr_count(&self) -> usize {
        self.controller.msr_count()
    }

    #[must_use]
    pub const fn bar0(&self) -> u64 {
        self.device.bar0
    }

    #[must_use]
    pub const fn backing_len(&self) -> usize {
        VIRTIO_BLK_BACKING_SIZE
    }

    pub fn encode(&self) -> Result<Vec<u8>, VersionedFullControllerVirtioBlkCheckpointError> {
        self.device.validate()?;
        let controller = self.controller.encode()?;
        let device = encode_device_state(&self.device)?;
        let total_len = FULL_CONTROLLER_VIRTIO_BLK_HEADER_LEN
            .checked_add(controller.len())
            .and_then(|length| length.checked_add(device.len()))
            .ok_or(VersionedFullControllerVirtioBlkCheckpointError::LengthOverflow)?;

        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_MAGIC);
        bytes.extend_from_slice(&VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_VERSION.to_le_bytes());
        bytes.extend_from_slice(&VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_ARCH_X86_64.to_le_bytes());
        bytes.extend_from_slice(&(FULL_CONTROLLER_VIRTIO_BLK_HEADER_LEN as u32).to_le_bytes());
        bytes.extend_from_slice(&(total_len as u64).to_le_bytes());
        bytes.extend_from_slice(&(controller.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(device.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&controller);
        bytes.extend_from_slice(&device);
        debug_assert_eq!(bytes.len(), total_len);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, VersionedFullControllerVirtioBlkCheckpointError> {
        if bytes.len() < FULL_CONTROLLER_VIRTIO_BLK_HEADER_LEN {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidTotalLength {
                declared: 0,
                actual: bytes.len(),
            });
        }
        if bytes[0..8] != VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_MAGIC {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version field"));
        if version != VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_VERSION {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::UnsupportedVersion(version));
        }
        let architecture =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed architecture field"));
        if architecture != VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_ARCH_X86_64 {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::UnsupportedArchitecture(
                architecture,
            ));
        }
        let header_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header length field"));
        if header_len != FULL_CONTROLLER_VIRTIO_BLK_HEADER_LEN as u32 {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidHeaderLength(
                header_len,
            ));
        }
        let declared_total =
            u64::from_le_bytes(bytes[16..24].try_into().expect("fixed total length field"));
        if declared_total != bytes.len() as u64 {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: bytes.len(),
            });
        }
        let controller_len =
            u64::from_le_bytes(bytes[24..32].try_into().expect("fixed controller length field"));
        let device_len =
            u64::from_le_bytes(bytes[32..40].try_into().expect("fixed device length field"));
        let flags = u32::from_le_bytes(bytes[40..44].try_into().expect("fixed flags field"));
        if flags != 0 {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::NonZeroFlags(flags));
        }
        let reserved =
            u32::from_le_bytes(bytes[44..48].try_into().expect("fixed reserved field"));
        if reserved != 0 {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::NonZeroReserved(
                reserved,
            ));
        }

        let controller_len = usize::try_from(controller_len).map_err(|_| {
            VersionedFullControllerVirtioBlkCheckpointError::InvalidControllerLength(controller_len)
        })?;
        if controller_len == 0 {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidControllerLength(0));
        }
        if device_len != VIRTIO_BLK_STATE_LEN as u64 {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidDeviceLength(
                device_len,
            ));
        }
        let device_len = usize::try_from(device_len)
            .map_err(|_| VersionedFullControllerVirtioBlkCheckpointError::InvalidDeviceLength(device_len))?;
        let expected_len = FULL_CONTROLLER_VIRTIO_BLK_HEADER_LEN
            .checked_add(controller_len)
            .and_then(|length| length.checked_add(device_len))
            .ok_or(VersionedFullControllerVirtioBlkCheckpointError::LengthOverflow)?;
        if expected_len != bytes.len() {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: expected_len,
            });
        }

        let controller_start = FULL_CONTROLLER_VIRTIO_BLK_HEADER_LEN;
        let controller_end = controller_start + controller_len;
        let controller =
            VersionedFullControllerCheckpointV1::decode(&bytes[controller_start..controller_end])?;
        let device = decode_device_state(&bytes[controller_end..])?;
        Ok(Self { controller, device })
    }

    pub fn materialize(
        &self,
        host_msrs: &crate::kvm::msr::HostMsrIndexList,
    ) -> Result<BoundedFullControllerVirtioBlkCheckpoint, VersionedFullControllerVirtioBlkCheckpointError>
    {
        let controller = self.controller.materialize(host_msrs)?;
        let device = self.device.materialize()?;
        if device.bar0() != self.device.bar0 {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::BarMismatch {
                checkpoint: self.device.bar0,
                device: device.bar0(),
            });
        }
        Ok(BoundedFullControllerVirtioBlkCheckpoint {
            controller,
            bar0: self.device.bar0,
            device,
        })
    }
}

fn encode_device_state(
    state: &VirtioBlkCheckpointState,
) -> Result<Vec<u8>, VersionedFullControllerVirtioBlkCheckpointError> {
    state.validate()?;
    let mut bytes = Vec::with_capacity(VIRTIO_BLK_STATE_LEN);
    bytes.extend_from_slice(&state.bar0.to_le_bytes());
    bytes.extend_from_slice(&state.device_feature_select.to_le_bytes());
    bytes.extend_from_slice(&state.driver_feature_select.to_le_bytes());
    bytes.extend_from_slice(&state.driver_features.to_le_bytes());
    bytes.push(state.status);
    bytes.push(u8::from(state.queue_enabled));
    bytes.push(state.isr_status);
    bytes.push(0);
    bytes.extend_from_slice(&state.queue_select.to_le_bytes());
    bytes.extend_from_slice(&state.queue_size.to_le_bytes());
    bytes.extend_from_slice(&state.last_avail_idx.to_le_bytes());
    bytes.extend_from_slice(&state.last_used_idx.to_le_bytes());
    bytes.extend_from_slice(&(VIRTIO_BLK_SECTOR_SIZE as u32).to_le_bytes());
    bytes.extend_from_slice(&VIRTIO_BLK_CAPACITY_SECTORS.to_le_bytes());
    bytes.extend_from_slice(&state.queue_desc.to_le_bytes());
    bytes.extend_from_slice(&state.queue_driver.to_le_bytes());
    bytes.extend_from_slice(&state.queue_device.to_le_bytes());
    bytes.extend_from_slice(&(VIRTIO_BLK_BACKING_SIZE as u32).to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    debug_assert_eq!(bytes.len(), VIRTIO_BLK_STATE_HEADER_LEN);
    bytes.extend_from_slice(&state.backing);
    debug_assert_eq!(bytes.len(), VIRTIO_BLK_STATE_LEN);
    Ok(bytes)
}

fn decode_device_state(
    bytes: &[u8],
) -> Result<VirtioBlkCheckpointState, VersionedFullControllerVirtioBlkCheckpointError> {
    if bytes.len() != VIRTIO_BLK_STATE_LEN {
        return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidDeviceLength(
            bytes.len() as u64,
        ));
    }
    let queue_enabled = match bytes[25] {
        0 => false,
        1 => true,
        value => {
            return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidQueueEnabled(
                value,
            ))
        }
    };
    if bytes[27] != 0 {
        return Err(VersionedFullControllerVirtioBlkCheckpointError::NonZeroDeviceReserved(
            u32::from(bytes[27]),
        ));
    }
    let sector_size = u32::from_le_bytes(bytes[36..40].try_into().expect("fixed sector-size field"));
    if sector_size != VIRTIO_BLK_SECTOR_SIZE as u32 {
        return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidSectorSize(
            sector_size,
        ));
    }
    let capacity = u64::from_le_bytes(bytes[40..48].try_into().expect("fixed capacity field"));
    if capacity != VIRTIO_BLK_CAPACITY_SECTORS {
        return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidCapacity(
            capacity,
        ));
    }
    let backing_len =
        u32::from_le_bytes(bytes[72..76].try_into().expect("fixed backing-length field"));
    if backing_len != VIRTIO_BLK_BACKING_SIZE as u32 {
        return Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidBackingLength(
            backing_len,
        ));
    }
    let reserved =
        u32::from_le_bytes(bytes[76..80].try_into().expect("fixed device reserved field"));
    if reserved != 0 {
        return Err(VersionedFullControllerVirtioBlkCheckpointError::NonZeroDeviceReserved(
            reserved,
        ));
    }

    let state = VirtioBlkCheckpointState {
        bar0: u64::from_le_bytes(bytes[0..8].try_into().expect("fixed BAR field")),
        device_feature_select: u32::from_le_bytes(
            bytes[8..12].try_into().expect("fixed device feature selector"),
        ),
        driver_feature_select: u32::from_le_bytes(
            bytes[12..16].try_into().expect("fixed driver feature selector"),
        ),
        driver_features: u64::from_le_bytes(
            bytes[16..24].try_into().expect("fixed driver features"),
        ),
        status: bytes[24],
        queue_enabled,
        isr_status: bytes[26],
        queue_select: u16::from_le_bytes(bytes[28..30].try_into().expect("fixed queue selector")),
        queue_size: u16::from_le_bytes(bytes[30..32].try_into().expect("fixed queue size")),
        last_avail_idx: u16::from_le_bytes(
            bytes[32..34].try_into().expect("fixed avail index"),
        ),
        last_used_idx: u16::from_le_bytes(
            bytes[34..36].try_into().expect("fixed used index"),
        ),
        queue_desc: u64::from_le_bytes(bytes[48..56].try_into().expect("fixed descriptor address")),
        queue_driver: u64::from_le_bytes(bytes[56..64].try_into().expect("fixed driver address")),
        queue_device: u64::from_le_bytes(bytes[64..72].try_into().expect("fixed device address")),
        backing: bytes[VIRTIO_BLK_STATE_HEADER_LEN..]
            .try_into()
            .expect("validated fixed backing length"),
    };
    state.validate()?;
    Ok(state)
}

#[cfg(test)]
mod versioned_full_controller_virtio_blk_schema_tests {
    use super::*;

    #[test]
    fn fixed_device_schema_size_is_model_bound() {
        assert_eq!(VIRTIO_BLK_STATE_HEADER_LEN, 80);
        assert_eq!(
            VIRTIO_BLK_STATE_LEN,
            80 + VIRTIO_BLK_SECTOR_SIZE * VIRTIO_BLK_CAPACITY_SECTORS as usize
        );
    }

    #[test]
    fn device_schema_rejects_model_and_reserved_corruption() {
        let device = crate::portio::pci::virtio_blk::VirtioBlkDevice::new(0x1000_0000);
        let state = VirtioBlkCheckpointState::capture(&device).unwrap();
        let bytes = encode_device_state(&state).unwrap();

        let mut bad_bool = bytes.clone();
        bad_bool[25] = 2;
        assert!(matches!(
            decode_device_state(&bad_bool),
            Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidQueueEnabled(2))
        ));

        let mut bad_sector = bytes.clone();
        bad_sector[36..40].copy_from_slice(&1024_u32.to_le_bytes());
        assert!(matches!(
            decode_device_state(&bad_sector),
            Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidSectorSize(1024))
        ));

        let mut bad_capacity = bytes.clone();
        bad_capacity[40..48].copy_from_slice(&5_u64.to_le_bytes());
        assert!(matches!(
            decode_device_state(&bad_capacity),
            Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidCapacity(5))
        ));

        let mut bad_reserved = bytes;
        bad_reserved[76..80].copy_from_slice(&1_u32.to_le_bytes());
        assert!(matches!(
            decode_device_state(&bad_reserved),
            Err(VersionedFullControllerVirtioBlkCheckpointError::NonZeroDeviceReserved(1))
        ));
    }

    fn minimally_sized_outer_envelope() -> Vec<u8> {
        let controller_len = 1_usize;
        let total_len = FULL_CONTROLLER_VIRTIO_BLK_HEADER_LEN
            + controller_len
            + VIRTIO_BLK_STATE_LEN;
        let mut bytes = vec![0_u8; total_len];
        bytes[0..8].copy_from_slice(&VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_MAGIC);
        bytes[8..10].copy_from_slice(&VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_VERSION.to_le_bytes());
        bytes[10..12]
            .copy_from_slice(&VERSIONED_FULL_CONTROLLER_VIRTIO_BLK_ARCH_X86_64.to_le_bytes());
        bytes[12..16].copy_from_slice(&(FULL_CONTROLLER_VIRTIO_BLK_HEADER_LEN as u32).to_le_bytes());
        bytes[16..24].copy_from_slice(&(total_len as u64).to_le_bytes());
        bytes[24..32].copy_from_slice(&(controller_len as u64).to_le_bytes());
        bytes[32..40].copy_from_slice(&(VIRTIO_BLK_STATE_LEN as u64).to_le_bytes());
        bytes
    }

    #[test]
    fn outer_header_fails_closed_before_nested_decode() {
        let base = minimally_sized_outer_envelope();

        let mut bad_magic = base.clone();
        bad_magic[0..8].copy_from_slice(b"BADFCVB!");
        assert_eq!(
            VersionedFullControllerVirtioBlkCheckpointV1::decode(&bad_magic),
            Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidMagic)
        );

        let mut bad_version = base.clone();
        bad_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedFullControllerVirtioBlkCheckpointV1::decode(&bad_version),
            Err(VersionedFullControllerVirtioBlkCheckpointError::UnsupportedVersion(2))
        );

        let mut bad_arch = base.clone();
        bad_arch[10..12].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedFullControllerVirtioBlkCheckpointV1::decode(&bad_arch),
            Err(VersionedFullControllerVirtioBlkCheckpointError::UnsupportedArchitecture(2))
        );

        let mut bad_header = base.clone();
        bad_header[12..16].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            VersionedFullControllerVirtioBlkCheckpointV1::decode(&bad_header),
            Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidHeaderLength(0))
        );

        let mut bad_total = base.clone();
        bad_total[16..24].copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            VersionedFullControllerVirtioBlkCheckpointV1::decode(&bad_total),
            Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidTotalLength {
                declared: 0,
                actual: base.len(),
            })
        );

        let mut bad_controller = base.clone();
        bad_controller[24..32].copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            VersionedFullControllerVirtioBlkCheckpointV1::decode(&bad_controller),
            Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidControllerLength(0))
        );

        let mut bad_device = base.clone();
        bad_device[32..40].copy_from_slice(&1_u64.to_le_bytes());
        assert_eq!(
            VersionedFullControllerVirtioBlkCheckpointV1::decode(&bad_device),
            Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidDeviceLength(1))
        );

        let mut bad_flags = base.clone();
        bad_flags[40..44].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedFullControllerVirtioBlkCheckpointV1::decode(&bad_flags),
            Err(VersionedFullControllerVirtioBlkCheckpointError::NonZeroFlags(1))
        );

        let mut bad_reserved = base.clone();
        bad_reserved[44..48].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedFullControllerVirtioBlkCheckpointV1::decode(&bad_reserved),
            Err(VersionedFullControllerVirtioBlkCheckpointError::NonZeroReserved(1))
        );

        let mut truncated = base;
        truncated.pop();
        assert!(matches!(
            VersionedFullControllerVirtioBlkCheckpointV1::decode(&truncated),
            Err(VersionedFullControllerVirtioBlkCheckpointError::InvalidTotalLength { .. })
        ));
    }
}
