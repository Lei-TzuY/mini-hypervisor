use crate::portio::pci::virtio_blk::{VirtioBlkCheckpointState, VIRTIO_BLK_BAR_SIZE};
use std::fmt;

pub const VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_MAGIC: [u8; 8] = *b"MHVFC2B\0";
pub const VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_VERSION: u16 = 1;
pub const VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_ARCH_X86_64: u16 = 1;

const FULL_CONTROLLER_TWO_VIRTIO_BLK_HEADER_LEN: usize = 56;
const FULL_CONTROLLER_TWO_VIRTIO_BLK_DEVICE_COUNT: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionedFullControllerTwoVirtioBlkCheckpointError {
    InvalidMagic,
    UnsupportedVersion(u16),
    UnsupportedArchitecture(u16),
    InvalidHeaderLength(u32),
    InvalidTotalLength { declared: u64, actual: usize },
    InvalidControllerLength(u64),
    InvalidDeviceLength(u64),
    InvalidDeviceCount(u32),
    NonZeroFlags(u32),
    NonZeroReserved(u64),
    LengthOverflow,
    NonCanonicalBars { first: u64, second: u64 },
    Controller(VersionedFullControllerCheckpointError),
    Device(VersionedFullControllerVirtioBlkCheckpointError),
}

impl fmt::Display for VersionedFullControllerTwoVirtioBlkCheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "two-device checkpoint magic does not match MHVFC2B"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported two-device checkpoint version {version}")
            }
            Self::UnsupportedArchitecture(architecture) => write!(
                f,
                "unsupported two-device checkpoint architecture identifier {architecture}"
            ),
            Self::InvalidHeaderLength(length) => write!(
                f,
                "two-device checkpoint header length {length} is not {FULL_CONTROLLER_TWO_VIRTIO_BLK_HEADER_LEN}"
            ),
            Self::InvalidTotalLength { declared, actual } => write!(
                f,
                "two-device checkpoint declares total length {declared}, actual byte length is {actual}"
            ),
            Self::InvalidControllerLength(length) => write!(
                f,
                "two-device checkpoint nested controller length {length} is invalid"
            ),
            Self::InvalidDeviceLength(length) => write!(
                f,
                "two-device checkpoint device payload length {length} is not {VIRTIO_BLK_STATE_LEN}"
            ),
            Self::InvalidDeviceCount(count) => {
                write!(f, "two-device checkpoint device count {count} is not 2")
            }
            Self::NonZeroFlags(flags) => {
                write!(f, "two-device checkpoint v1 flags must be zero, got {flags:#x}")
            }
            Self::NonZeroReserved(value) => {
                write!(f, "two-device checkpoint reserved field must be zero, got {value:#x}")
            }
            Self::LengthOverflow => write!(f, "two-device checkpoint length arithmetic overflowed"),
            Self::NonCanonicalBars { first, second } => write!(
                f,
                "two-device checkpoint BARs must be distinct, aligned and strictly increasing, got {first:#x}, {second:#x}"
            ),
            Self::Controller(error) => write!(f, "nested full-controller checkpoint is invalid: {error}"),
            Self::Device(error) => write!(f, "nested virtio-blk checkpoint state is invalid: {error}"),
        }
    }
}

impl std::error::Error for VersionedFullControllerTwoVirtioBlkCheckpointError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Controller(error) => Some(error),
            Self::Device(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VersionedFullControllerCheckpointError>
    for VersionedFullControllerTwoVirtioBlkCheckpointError
{
    fn from(error: VersionedFullControllerCheckpointError) -> Self {
        Self::Controller(error)
    }
}

impl From<VersionedFullControllerVirtioBlkCheckpointError>
    for VersionedFullControllerTwoVirtioBlkCheckpointError
{
    fn from(error: VersionedFullControllerVirtioBlkCheckpointError) -> Self {
        Self::Device(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedFullControllerTwoVirtioBlkCheckpointV1 {
    controller: VersionedFullControllerCheckpointV1,
    devices: [VirtioBlkCheckpointState; 2],
}

impl VersionedFullControllerTwoVirtioBlkCheckpointV1 {
    pub fn from_checkpoint(
        checkpoint: &BoundedFullControllerTwoVirtioBlkCheckpoint,
    ) -> Result<Self, VersionedFullControllerTwoVirtioBlkCheckpointError> {
        let controller = VersionedFullControllerCheckpointV1::from_checkpoint(checkpoint.controller())?;
        let bars = checkpoint.device_bars();
        validate_two_device_bars(bars)?;
        let first = VirtioBlkCheckpointState::capture(
            checkpoint
                .device(bars[0])
                .expect("checkpoint BAR set owns the first device"),
        )
        .map_err(|error| Self::map_device_state_error(error))?;
        let second = VirtioBlkCheckpointState::capture(
            checkpoint
                .device(bars[1])
                .expect("checkpoint BAR set owns the second device"),
        )
        .map_err(|error| Self::map_device_state_error(error))?;
        validate_two_device_bars([first.bar0, second.bar0])?;
        Ok(Self {
            controller,
            devices: [first, second],
        })
    }

    fn map_device_state_error(
        error: crate::portio::pci::virtio_blk::VirtioBlkCheckpointStateError,
    ) -> VersionedFullControllerTwoVirtioBlkCheckpointError {
        VersionedFullControllerTwoVirtioBlkCheckpointError::Device(
            VersionedFullControllerVirtioBlkCheckpointError::Device(error),
        )
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_VERSION
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
    pub const fn device_bars(&self) -> [u64; 2] {
        [self.devices[0].bar0, self.devices[1].bar0]
    }

    #[must_use]
    pub const fn backing_len_each(&self) -> usize {
        VIRTIO_BLK_BACKING_SIZE
    }

    pub fn encode(
        &self,
    ) -> Result<Vec<u8>, VersionedFullControllerTwoVirtioBlkCheckpointError> {
        validate_two_device_bars(self.device_bars())?;
        let controller = self.controller.encode()?;
        let first = encode_device_state(&self.devices[0])?;
        let second = encode_device_state(&self.devices[1])?;
        let total_len = FULL_CONTROLLER_TWO_VIRTIO_BLK_HEADER_LEN
            .checked_add(controller.len())
            .and_then(|length| length.checked_add(first.len()))
            .and_then(|length| length.checked_add(second.len()))
            .ok_or(VersionedFullControllerTwoVirtioBlkCheckpointError::LengthOverflow)?;

        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_MAGIC);
        bytes.extend_from_slice(&VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_VERSION.to_le_bytes());
        bytes.extend_from_slice(&VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_ARCH_X86_64.to_le_bytes());
        bytes.extend_from_slice(&(FULL_CONTROLLER_TWO_VIRTIO_BLK_HEADER_LEN as u32).to_le_bytes());
        bytes.extend_from_slice(&(total_len as u64).to_le_bytes());
        bytes.extend_from_slice(&(controller.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(VIRTIO_BLK_STATE_LEN as u64).to_le_bytes());
        bytes.extend_from_slice(&FULL_CONTROLLER_TWO_VIRTIO_BLK_DEVICE_COUNT.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&controller);
        bytes.extend_from_slice(&first);
        bytes.extend_from_slice(&second);
        debug_assert_eq!(bytes.len(), total_len);
        Ok(bytes)
    }

    pub fn decode(
        bytes: &[u8],
    ) -> Result<Self, VersionedFullControllerTwoVirtioBlkCheckpointError> {
        if bytes.len() < FULL_CONTROLLER_TWO_VIRTIO_BLK_HEADER_LEN {
            return Err(
                VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidTotalLength {
                    declared: 0,
                    actual: bytes.len(),
                },
            );
        }
        if bytes[0..8] != VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_MAGIC {
            return Err(VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version field"));
        if version != VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_VERSION {
            return Err(
                VersionedFullControllerTwoVirtioBlkCheckpointError::UnsupportedVersion(version),
            );
        }
        let architecture =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed architecture field"));
        if architecture != VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_ARCH_X86_64 {
            return Err(
                VersionedFullControllerTwoVirtioBlkCheckpointError::UnsupportedArchitecture(
                    architecture,
                ),
            );
        }
        let header_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header length field"));
        if header_len != FULL_CONTROLLER_TWO_VIRTIO_BLK_HEADER_LEN as u32 {
            return Err(
                VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidHeaderLength(header_len),
            );
        }
        let declared_total =
            u64::from_le_bytes(bytes[16..24].try_into().expect("fixed total length field"));
        if usize::try_from(declared_total).ok() != Some(bytes.len()) {
            return Err(
                VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidTotalLength {
                    declared: declared_total,
                    actual: bytes.len(),
                },
            );
        }
        let controller_len =
            u64::from_le_bytes(bytes[24..32].try_into().expect("fixed controller length field"));
        let device_len =
            u64::from_le_bytes(bytes[32..40].try_into().expect("fixed device length field"));
        let device_count =
            u32::from_le_bytes(bytes[40..44].try_into().expect("fixed device count field"));
        if device_count != FULL_CONTROLLER_TWO_VIRTIO_BLK_DEVICE_COUNT {
            return Err(
                VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidDeviceCount(
                    device_count,
                ),
            );
        }
        let flags = u32::from_le_bytes(bytes[44..48].try_into().expect("fixed flags field"));
        if flags != 0 {
            return Err(VersionedFullControllerTwoVirtioBlkCheckpointError::NonZeroFlags(
                flags,
            ));
        }
        let reserved =
            u64::from_le_bytes(bytes[48..56].try_into().expect("fixed reserved field"));
        if reserved != 0 {
            return Err(
                VersionedFullControllerTwoVirtioBlkCheckpointError::NonZeroReserved(reserved),
            );
        }

        let controller_len = usize::try_from(controller_len).map_err(|_| {
            VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidControllerLength(
                controller_len,
            )
        })?;
        if controller_len == 0 {
            return Err(
                VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidControllerLength(0),
            );
        }
        if device_len != VIRTIO_BLK_STATE_LEN as u64 {
            return Err(
                VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidDeviceLength(device_len),
            );
        }
        let expected_len = FULL_CONTROLLER_TWO_VIRTIO_BLK_HEADER_LEN
            .checked_add(controller_len)
            .and_then(|length| length.checked_add(VIRTIO_BLK_STATE_LEN))
            .and_then(|length| length.checked_add(VIRTIO_BLK_STATE_LEN))
            .ok_or(VersionedFullControllerTwoVirtioBlkCheckpointError::LengthOverflow)?;
        if expected_len != bytes.len() {
            return Err(
                VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidTotalLength {
                    declared: declared_total,
                    actual: expected_len,
                },
            );
        }

        let controller_start = FULL_CONTROLLER_TWO_VIRTIO_BLK_HEADER_LEN;
        let controller_end = controller_start + controller_len;
        let first_end = controller_end + VIRTIO_BLK_STATE_LEN;
        let controller =
            VersionedFullControllerCheckpointV1::decode(&bytes[controller_start..controller_end])?;
        let first = decode_device_state(&bytes[controller_end..first_end])?;
        let second = decode_device_state(&bytes[first_end..])?;
        validate_two_device_bars([first.bar0, second.bar0])?;
        Ok(Self {
            controller,
            devices: [first, second],
        })
    }

    pub fn materialize(
        &self,
        host_msrs: &crate::kvm::msr::HostMsrIndexList,
    ) -> Result<
        BoundedFullControllerTwoVirtioBlkCheckpoint,
        VersionedFullControllerTwoVirtioBlkCheckpointError,
    > {
        validate_two_device_bars(self.device_bars())?;
        let controller = self.controller.materialize(host_msrs)?;
        let first = self.devices[0]
            .materialize()
            .map_err(Self::map_device_state_error)?;
        let second = self.devices[1]
            .materialize()
            .map_err(Self::map_device_state_error)?;
        Ok(BoundedFullControllerTwoVirtioBlkCheckpoint {
            controller,
            devices: [(self.devices[0].bar0, first), (self.devices[1].bar0, second)],
        })
    }
}

fn validate_two_device_bars(
    bars: [u64; 2],
) -> Result<(), VersionedFullControllerTwoVirtioBlkCheckpointError> {
    if bars[0] >= bars[1]
        || bars[0] % u64::from(crate::portio::pci::virtio_blk::VIRTIO_BLK_BAR_SIZE) != 0
        || bars[1] % u64::from(crate::portio::pci::virtio_blk::VIRTIO_BLK_BAR_SIZE) != 0
    {
        return Err(
            VersionedFullControllerTwoVirtioBlkCheckpointError::NonCanonicalBars {
                first: bars[0],
                second: bars[1],
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod versioned_full_controller_two_virtio_blk_schema_tests {
    use super::*;

    const FIRST_BAR: u64 = 0x1000_0000;
    const SECOND_BAR: u64 = FIRST_BAR + crate::portio::pci::virtio_blk::VIRTIO_BLK_BAR_SIZE as u64;

    fn minimal_envelope() -> Vec<u8> {
        let controller_len = 1_usize;
        let total_len = FULL_CONTROLLER_TWO_VIRTIO_BLK_HEADER_LEN
            + controller_len
            + 2 * VIRTIO_BLK_STATE_LEN;
        let mut bytes = vec![0_u8; total_len];
        bytes[0..8].copy_from_slice(&VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_MAGIC);
        bytes[8..10]
            .copy_from_slice(&VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_VERSION.to_le_bytes());
        bytes[10..12]
            .copy_from_slice(&VERSIONED_FULL_CONTROLLER_TWO_VIRTIO_BLK_ARCH_X86_64.to_le_bytes());
        bytes[12..16]
            .copy_from_slice(&(FULL_CONTROLLER_TWO_VIRTIO_BLK_HEADER_LEN as u32).to_le_bytes());
        bytes[16..24].copy_from_slice(&(total_len as u64).to_le_bytes());
        bytes[24..32].copy_from_slice(&(controller_len as u64).to_le_bytes());
        bytes[32..40].copy_from_slice(&(VIRTIO_BLK_STATE_LEN as u64).to_le_bytes());
        bytes[40..44].copy_from_slice(&FULL_CONTROLLER_TWO_VIRTIO_BLK_DEVICE_COUNT.to_le_bytes());
        bytes
    }

    #[test]
    fn two_device_bar_ownership_is_canonical() {
        assert!(validate_two_device_bars([FIRST_BAR, SECOND_BAR]).is_ok());
        assert!(matches!(
            validate_two_device_bars([SECOND_BAR, FIRST_BAR]),
            Err(VersionedFullControllerTwoVirtioBlkCheckpointError::NonCanonicalBars { .. })
        ));
        assert!(validate_two_device_bars([FIRST_BAR, FIRST_BAR]).is_err());
        assert!(validate_two_device_bars([FIRST_BAR + 1, SECOND_BAR]).is_err());
    }

    #[test]
    fn outer_header_rejects_wrong_device_shape_before_nested_decode() {
        let base = minimal_envelope();

        let mut bad_count = base.clone();
        bad_count[40..44].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedFullControllerTwoVirtioBlkCheckpointV1::decode(&bad_count),
            Err(VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidDeviceCount(1))
        );

        let mut bad_device_len = base.clone();
        bad_device_len[32..40].copy_from_slice(&1_u64.to_le_bytes());
        assert_eq!(
            VersionedFullControllerTwoVirtioBlkCheckpointV1::decode(&bad_device_len),
            Err(VersionedFullControllerTwoVirtioBlkCheckpointError::InvalidDeviceLength(1))
        );

        let mut bad_flags = base.clone();
        bad_flags[44..48].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedFullControllerTwoVirtioBlkCheckpointV1::decode(&bad_flags),
            Err(VersionedFullControllerTwoVirtioBlkCheckpointError::NonZeroFlags(1))
        );

        let mut bad_reserved = base;
        bad_reserved[48..56].copy_from_slice(&1_u64.to_le_bytes());
        assert_eq!(
            VersionedFullControllerTwoVirtioBlkCheckpointV1::decode(&bad_reserved),
            Err(VersionedFullControllerTwoVirtioBlkCheckpointError::NonZeroReserved(1))
        );
    }
}
