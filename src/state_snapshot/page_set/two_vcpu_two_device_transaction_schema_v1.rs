use super::{
    decode_device_state, encode_device_state, VersionedFullControllerVirtioBlkCheckpointError,
    VIRTIO_BLK_STATE_LEN,
};
use crate::kvm::sys::{
    VersionedHostRegistrationPairError, VersionedHostRegistrationPairV1,
    VERSIONED_HOST_REGISTRATION_PAIR_LEN,
};
use crate::portio::pci::virtio_blk::{
    VirtioBlkCheckpointState, VirtioBlkCheckpointStateError, VirtioBlkPendingCompletionToken,
    VIRTIO_BLK_BACKING_SIZE,
};
use crate::state_snapshot::{
    VersionedTwoVcpuFullControllerCheckpointError,
    VersionedTwoVcpuFullControllerCheckpointV1,
};
use crate::vcpu::VcpuId;

pub const VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_MAGIC: [u8; 8] = *b"MHV2V2B\0";
pub const VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_VERSION: u16 = 1;
pub const VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_ARCH_X86_64: u16 = 1;
const TWO_VCPU_TWO_DEVICE_CHECKPOINT_HEADER_LEN: usize = 56;
const TWO_VCPU_TWO_DEVICE_CHECKPOINT_DEVICE_COUNT: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionedTwoVcpuTwoDeviceCheckpointError {
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
    NonCanonicalBars { first: u64, second: u64 },
    LengthOverflow,
    Controller(VersionedTwoVcpuFullControllerCheckpointError),
    Device(VersionedFullControllerVirtioBlkCheckpointError),
}

impl std::fmt::Display for VersionedTwoVcpuTwoDeviceCheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "two-vCPU two-device checkpoint magic does not match MHV2V2B"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported two-vCPU two-device checkpoint version {version}"),
            Self::UnsupportedArchitecture(architecture) => write!(f, "unsupported two-vCPU two-device checkpoint architecture identifier {architecture}"),
            Self::InvalidHeaderLength(length) => write!(f, "two-vCPU two-device checkpoint header length {length} is not {TWO_VCPU_TWO_DEVICE_CHECKPOINT_HEADER_LEN}"),
            Self::InvalidTotalLength { declared, actual } => write!(f, "two-vCPU two-device checkpoint declares total length {declared}, actual byte length is {actual}"),
            Self::InvalidControllerLength(length) => write!(f, "two-vCPU two-device checkpoint nested controller length {length} is invalid"),
            Self::InvalidDeviceLength(length) => write!(f, "two-vCPU two-device checkpoint device payload length {length} is not {VIRTIO_BLK_STATE_LEN}"),
            Self::InvalidDeviceCount(count) => write!(f, "two-vCPU two-device checkpoint device count {count} is not 2"),
            Self::NonZeroFlags(flags) => write!(f, "two-vCPU two-device checkpoint v1 flags must be zero, got {flags:#x}"),
            Self::NonZeroReserved(value) => write!(f, "two-vCPU two-device checkpoint reserved field must be zero, got {value:#x}"),
            Self::NonCanonicalBars { first, second } => write!(f, "two-vCPU two-device checkpoint BARs must be distinct, aligned and strictly increasing, got {first:#x}, {second:#x}"),
            Self::LengthOverflow => write!(f, "two-vCPU two-device checkpoint length arithmetic overflowed"),
            Self::Controller(error) => write!(f, "nested two-vCPU controller checkpoint is invalid: {error}"),
            Self::Device(error) => write!(f, "nested virtio-blk checkpoint is invalid: {error}"),
        }
    }
}

impl std::error::Error for VersionedTwoVcpuTwoDeviceCheckpointError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Controller(error) => Some(error),
            Self::Device(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VersionedTwoVcpuFullControllerCheckpointError>
    for VersionedTwoVcpuTwoDeviceCheckpointError
{
    fn from(error: VersionedTwoVcpuFullControllerCheckpointError) -> Self {
        Self::Controller(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionedTwoVcpuTwoDeviceCheckpointV1 {
    controller: VersionedTwoVcpuFullControllerCheckpointV1,
    devices: [VirtioBlkCheckpointState; 2],
}

impl VersionedTwoVcpuTwoDeviceCheckpointV1 {
    pub(crate) fn from_checkpoint(
        checkpoint: &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint,
    ) -> Result<Self, VersionedTwoVcpuTwoDeviceCheckpointError> {
        let controller =
            VersionedTwoVcpuFullControllerCheckpointV1::from_checkpoint(checkpoint.controller())?;
        let bars = checkpoint.device_bars();
        validate_versioned_two_device_bars(bars)?;
        let first = VirtioBlkCheckpointState::capture(
            checkpoint
                .device(bars[0])
                .expect("canonical checkpoint owns first device"),
        )
        .map_err(Self::map_device_state_error)?;
        let second = VirtioBlkCheckpointState::capture(
            checkpoint
                .device(bars[1])
                .expect("canonical checkpoint owns second device"),
        )
        .map_err(Self::map_device_state_error)?;
        validate_versioned_two_device_bars([first.bar0, second.bar0])?;
        Ok(Self {
            controller,
            devices: [first, second],
        })
    }

    pub(crate) fn from_checkpoint_with_pending_completion(
        checkpoint: &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint,
        token: &VirtioBlkPendingCompletionToken,
    ) -> Result<Self, VersionedTwoVcpuTwoDeviceCheckpointError> {
        let controller =
            VersionedTwoVcpuFullControllerCheckpointV1::from_checkpoint(checkpoint.controller())?;
        let bars = checkpoint.device_bars();
        validate_versioned_two_device_bars(bars)?;
        if token.bar0() != bars[0] && token.bar0() != bars[1] {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::Device(
                VersionedFullControllerVirtioBlkCheckpointError::Device(
                    token.validate_device(
                        checkpoint
                            .device(bars[0])
                            .expect("canonical checkpoint owns first device"),
                    )
                    .unwrap_err(),
                ),
            ));
        }

        let mut captured_token_matches = false;
        let first = if token.bar0() == bars[0] {
            let (state, captured) = VirtioBlkCheckpointState::capture_with_pending_completion(
                checkpoint
                    .device(bars[0])
                    .expect("canonical checkpoint owns first device"),
            )
            .map_err(Self::map_device_state_error)?;
            captured_token_matches = &captured == token;
            state
        } else {
            VirtioBlkCheckpointState::capture(
                checkpoint
                    .device(bars[0])
                    .expect("canonical checkpoint owns first device"),
            )
            .map_err(Self::map_device_state_error)?
        };
        let second = if token.bar0() == bars[1] {
            let (state, captured) = VirtioBlkCheckpointState::capture_with_pending_completion(
                checkpoint
                    .device(bars[1])
                    .expect("canonical checkpoint owns second device"),
            )
            .map_err(Self::map_device_state_error)?;
            captured_token_matches = &captured == token;
            state
        } else {
            VirtioBlkCheckpointState::capture(
                checkpoint
                    .device(bars[1])
                    .expect("canonical checkpoint owns second device"),
            )
            .map_err(Self::map_device_state_error)?
        };
        if !captured_token_matches {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::Device(
                VersionedFullControllerVirtioBlkCheckpointError::Device(
                    VirtioBlkCheckpointStateError::InvalidPendingCompletionToken {
                        bar0: token.bar0(),
                        queue: token.queue(),
                        last_avail_idx: token.last_avail_idx(),
                        last_used_idx: token.last_used_idx(),
                    },
                ),
            ));
        }
        token
            .validate_state(if token.bar0() == bars[0] { &first } else { &second })
            .map_err(Self::map_device_state_error)?;
        Ok(Self {
            controller,
            devices: [first, second],
        })
    }

    pub(crate) fn validate_pending_completion_token(
        &self,
        token: &VirtioBlkPendingCompletionToken,
    ) -> Result<(), VersionedTwoVcpuTwoDeviceCheckpointError> {
        let bars = self.device_bars();
        let state = if token.bar0() == bars[0] {
            &self.devices[0]
        } else if token.bar0() == bars[1] {
            &self.devices[1]
        } else {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::Device(
                VersionedFullControllerVirtioBlkCheckpointError::Device(
                    VirtioBlkCheckpointStateError::InvalidPendingCompletionToken {
                        bar0: token.bar0(),
                        queue: token.queue(),
                        last_avail_idx: token.last_avail_idx(),
                        last_used_idx: token.last_used_idx(),
                    },
                ),
            ));
        };
        token.validate_state(state).map_err(Self::map_device_state_error)
    }

    fn map_device_state_error(
        error: VirtioBlkCheckpointStateError,
    ) -> VersionedTwoVcpuTwoDeviceCheckpointError {
        VersionedTwoVcpuTwoDeviceCheckpointError::Device(
            VersionedFullControllerVirtioBlkCheckpointError::Device(error),
        )
    }

    #[must_use]
    pub(crate) const fn version(&self) -> u16 {
        VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_VERSION
    }

    #[must_use]
    pub(crate) const fn controller_version(&self) -> u16 {
        self.controller.version()
    }

    #[must_use]
    pub(crate) const fn vcpu_ids(&self) -> [VcpuId; 2] {
        self.controller.vcpu_ids()
    }

    #[must_use]
    pub(crate) const fn mp_states(&self) -> [u32; 2] {
        self.controller.mp_states()
    }

    #[must_use]
    pub(crate) fn page_count(&self) -> usize {
        self.controller.page_count()
    }

    #[must_use]
    pub(crate) fn msr_counts(&self) -> [usize; 2] {
        self.controller.msr_counts()
    }

    #[must_use]
    pub(crate) const fn device_bars(&self) -> [u64; 2] {
        [self.devices[0].bar0, self.devices[1].bar0]
    }

    #[must_use]
    pub(crate) const fn backing_len_each(&self) -> usize {
        VIRTIO_BLK_BACKING_SIZE
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, VersionedTwoVcpuTwoDeviceCheckpointError> {
        validate_versioned_two_device_bars(self.device_bars())?;
        let controller = self.controller.encode()?;
        let first = encode_device_state(&self.devices[0])
            .map_err(VersionedTwoVcpuTwoDeviceCheckpointError::Device)?;
        let second = encode_device_state(&self.devices[1])
            .map_err(VersionedTwoVcpuTwoDeviceCheckpointError::Device)?;
        let total_len = TWO_VCPU_TWO_DEVICE_CHECKPOINT_HEADER_LEN
            .checked_add(controller.len())
            .and_then(|length| length.checked_add(first.len()))
            .and_then(|length| length.checked_add(second.len()))
            .ok_or(VersionedTwoVcpuTwoDeviceCheckpointError::LengthOverflow)?;

        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_MAGIC);
        bytes.extend_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_ARCH_X86_64.to_le_bytes());
        bytes.extend_from_slice(&(TWO_VCPU_TWO_DEVICE_CHECKPOINT_HEADER_LEN as u32).to_le_bytes());
        bytes.extend_from_slice(&(total_len as u64).to_le_bytes());
        bytes.extend_from_slice(&(controller.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(VIRTIO_BLK_STATE_LEN as u64).to_le_bytes());
        bytes.extend_from_slice(&TWO_VCPU_TWO_DEVICE_CHECKPOINT_DEVICE_COUNT.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes.extend_from_slice(&controller);
        bytes.extend_from_slice(&first);
        bytes.extend_from_slice(&second);
        debug_assert_eq!(bytes.len(), total_len);
        Ok(bytes)
    }

    pub(crate) fn decode(
        bytes: &[u8],
    ) -> Result<Self, VersionedTwoVcpuTwoDeviceCheckpointError> {
        if bytes.len() < TWO_VCPU_TWO_DEVICE_CHECKPOINT_HEADER_LEN {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidTotalLength {
                declared: 0,
                actual: bytes.len(),
            });
        }
        if bytes[0..8] != VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_MAGIC {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version field"));
        if version != VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_VERSION {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::UnsupportedVersion(version));
        }
        let architecture =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed architecture field"));
        if architecture != VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_ARCH_X86_64 {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::UnsupportedArchitecture(
                architecture,
            ));
        }
        let header_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header length field"));
        if header_len != TWO_VCPU_TWO_DEVICE_CHECKPOINT_HEADER_LEN as u32 {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidHeaderLength(
                header_len,
            ));
        }
        let declared_total =
            u64::from_le_bytes(bytes[16..24].try_into().expect("fixed total length field"));
        if usize::try_from(declared_total).ok() != Some(bytes.len()) {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: bytes.len(),
            });
        }
        let controller_len =
            u64::from_le_bytes(bytes[24..32].try_into().expect("fixed controller length field"));
        let device_len =
            u64::from_le_bytes(bytes[32..40].try_into().expect("fixed device length field"));
        let device_count =
            u32::from_le_bytes(bytes[40..44].try_into().expect("fixed device count field"));
        if device_count != TWO_VCPU_TWO_DEVICE_CHECKPOINT_DEVICE_COUNT {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidDeviceCount(
                device_count,
            ));
        }
        let flags = u32::from_le_bytes(bytes[44..48].try_into().expect("fixed flags field"));
        if flags != 0 {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::NonZeroFlags(flags));
        }
        let reserved =
            u64::from_le_bytes(bytes[48..56].try_into().expect("fixed reserved field"));
        if reserved != 0 {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::NonZeroReserved(
                reserved,
            ));
        }
        let controller_len = usize::try_from(controller_len).map_err(|_| {
            VersionedTwoVcpuTwoDeviceCheckpointError::InvalidControllerLength(controller_len)
        })?;
        if controller_len == 0 {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidControllerLength(0));
        }
        if device_len != VIRTIO_BLK_STATE_LEN as u64 {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidDeviceLength(
                device_len,
            ));
        }
        let expected_len = TWO_VCPU_TWO_DEVICE_CHECKPOINT_HEADER_LEN
            .checked_add(controller_len)
            .and_then(|length| length.checked_add(VIRTIO_BLK_STATE_LEN))
            .and_then(|length| length.checked_add(VIRTIO_BLK_STATE_LEN))
            .ok_or(VersionedTwoVcpuTwoDeviceCheckpointError::LengthOverflow)?;
        if expected_len != bytes.len() {
            return Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: expected_len,
            });
        }
        let controller_start = TWO_VCPU_TWO_DEVICE_CHECKPOINT_HEADER_LEN;
        let controller_end = controller_start + controller_len;
        let first_end = controller_end + VIRTIO_BLK_STATE_LEN;
        let controller = VersionedTwoVcpuFullControllerCheckpointV1::decode(
            &bytes[controller_start..controller_end],
        )?;
        let first = decode_device_state(&bytes[controller_end..first_end])
            .map_err(VersionedTwoVcpuTwoDeviceCheckpointError::Device)?;
        let second = decode_device_state(&bytes[first_end..])
            .map_err(VersionedTwoVcpuTwoDeviceCheckpointError::Device)?;
        validate_versioned_two_device_bars([first.bar0, second.bar0])?;
        Ok(Self {
            controller,
            devices: [first, second],
        })
    }

    pub(crate) fn materialize(
        &self,
        host_msrs: &crate::kvm::msr::HostMsrIndexList,
    ) -> Result<
        BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint,
        VersionedTwoVcpuTwoDeviceCheckpointError,
    > {
        validate_versioned_two_device_bars(self.device_bars())?;
        let controller = self.controller.materialize(host_msrs)?;
        let first = self.devices[0]
            .materialize()
            .map_err(Self::map_device_state_error)?;
        let second = self.devices[1]
            .materialize()
            .map_err(Self::map_device_state_error)?;
        Ok(BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint {
            controller,
            devices: [
                (self.devices[0].bar0, first),
                (self.devices[1].bar0, second),
            ],
        })
    }
}

fn validate_versioned_two_device_bars(
    bars: [u64; 2],
) -> Result<(), VersionedTwoVcpuTwoDeviceCheckpointError> {
    let alignment = u64::from(crate::portio::pci::virtio_blk::VIRTIO_BLK_BAR_SIZE);
    if bars[0] >= bars[1] || bars[0] % alignment != 0 || bars[1] % alignment != 0 {
        return Err(VersionedTwoVcpuTwoDeviceCheckpointError::NonCanonicalBars {
            first: bars[0],
            second: bars[1],
        });
    }
    Ok(())
}

pub const VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_MAGIC: [u8; 8] = *b"MHV2TX\0\0";
pub const VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_VERSION: u16 = 1;
pub const VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_ARCH_X86_64: u16 = 1;
const TWO_VCPU_TWO_DEVICE_TRANSACTION_HEADER_LEN: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VersionedTwoVcpuTwoDeviceTransactionError {
    InvalidMagic,
    UnsupportedVersion(u16),
    UnsupportedArchitecture(u16),
    InvalidHeaderLength(u32),
    InvalidTotalLength { declared: u64, actual: usize },
    InvalidCheckpointLength(u64),
    InvalidRegistrationPairLength(u64),
    InvalidPendingCompletionTokenLength(u64),
    PendingCompletion(String),
    NonZeroFlags(u32),
    NonZeroReserved(u32),
    RegistrationBinding(String),
    LengthOverflow,
    Checkpoint(VersionedTwoVcpuTwoDeviceCheckpointError),
    RegistrationPair(VersionedHostRegistrationPairError),
}

impl std::fmt::Display for VersionedTwoVcpuTwoDeviceTransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "two-vCPU two-device transaction magic does not match MHV2TX"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported two-vCPU two-device transaction version {version}"),
            Self::UnsupportedArchitecture(architecture) => write!(f, "unsupported two-vCPU two-device transaction architecture identifier {architecture}"),
            Self::InvalidHeaderLength(length) => write!(f, "two-vCPU two-device transaction header length {length} is not {TWO_VCPU_TWO_DEVICE_TRANSACTION_HEADER_LEN}"),
            Self::InvalidTotalLength { declared, actual } => write!(f, "two-vCPU two-device transaction declares total length {declared}, actual byte length is {actual}"),
            Self::InvalidCheckpointLength(length) => write!(f, "two-vCPU two-device transaction checkpoint length {length} is invalid"),
            Self::InvalidRegistrationPairLength(length) => write!(f, "two-vCPU two-device transaction registration-pair length {length} is not {VERSIONED_HOST_REGISTRATION_PAIR_LEN}"),
            Self::InvalidPendingCompletionTokenLength(length) => write!(f, "two-vCPU two-device transaction pending-completion token length {length} is not {PENDING_COMPLETION_TOKEN_LEN}"),
            Self::PendingCompletion(detail) => write!(f, "two-vCPU two-device pending-completion token is invalid: {detail}"),
            Self::NonZeroFlags(flags) => write!(f, "two-vCPU two-device transaction v1 flags must be zero, got {flags:#x}"),
            Self::NonZeroReserved(value) => write!(f, "two-vCPU two-device transaction reserved field must be zero, got {value:#x}"),
            Self::RegistrationBinding(detail) => write!(f, "two-vCPU two-device transaction registration binding is invalid: {detail}"),
            Self::LengthOverflow => write!(f, "two-vCPU two-device transaction length arithmetic overflowed"),
            Self::Checkpoint(error) => write!(f, "nested two-vCPU two-device checkpoint is invalid: {error}"),
            Self::RegistrationPair(error) => write!(f, "nested host-registration pair is invalid: {error}"),
        }
    }
}

impl std::error::Error for VersionedTwoVcpuTwoDeviceTransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Checkpoint(error) => Some(error),
            Self::RegistrationPair(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VersionedTwoVcpuTwoDeviceCheckpointError>
    for VersionedTwoVcpuTwoDeviceTransactionError
{
    fn from(error: VersionedTwoVcpuTwoDeviceCheckpointError) -> Self {
        Self::Checkpoint(error)
    }
}

impl From<VersionedHostRegistrationPairError>
    for VersionedTwoVcpuTwoDeviceTransactionError
{
    fn from(error: VersionedHostRegistrationPairError) -> Self {
        Self::RegistrationPair(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionedTwoVcpuTwoDeviceTransactionV1 {
    checkpoint: VersionedTwoVcpuTwoDeviceCheckpointV1,
    registrations: VersionedHostRegistrationPairV1,
}

impl VersionedTwoVcpuTwoDeviceTransactionV1 {
    pub(crate) fn from_checkpoint_and_pair(
        checkpoint: &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint,
        pair: HostRegistrationSpecPair,
    ) -> Result<Self, VersionedTwoVcpuTwoDeviceTransactionError> {
        validate_transaction_registration_binding(checkpoint.device_bars(), pair)?;
        Ok(Self {
            checkpoint: VersionedTwoVcpuTwoDeviceCheckpointV1::from_checkpoint(checkpoint)?,
            registrations: VersionedHostRegistrationPairV1::from_pair(pair),
        })
    }

    #[must_use]
    pub(crate) const fn version(&self) -> u16 {
        VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_VERSION
    }

    #[must_use]
    pub(crate) const fn checkpoint_version(&self) -> u16 {
        self.checkpoint.version()
    }

    #[must_use]
    pub(crate) const fn controller_version(&self) -> u16 {
        self.checkpoint.controller_version()
    }

    #[must_use]
    pub(crate) const fn registration_pair_version(&self) -> u16 {
        self.registrations.version()
    }

    #[must_use]
    pub(crate) const fn registration_versions(&self) -> [u16; 2] {
        self.registrations.registration_versions()
    }

    #[must_use]
    pub(crate) const fn vcpu_ids(&self) -> [VcpuId; 2] {
        self.checkpoint.vcpu_ids()
    }

    #[must_use]
    pub(crate) const fn mp_states(&self) -> [u32; 2] {
        self.checkpoint.mp_states()
    }

    #[must_use]
    pub(crate) fn page_count(&self) -> usize {
        self.checkpoint.page_count()
    }

    #[must_use]
    pub(crate) fn msr_counts(&self) -> [usize; 2] {
        self.checkpoint.msr_counts()
    }

    #[must_use]
    pub(crate) const fn bars(&self) -> [u64; 2] {
        self.checkpoint.device_bars()
    }

    #[must_use]
    pub(crate) const fn backing_len_each(&self) -> usize {
        self.checkpoint.backing_len_each()
    }

    pub(crate) fn checkpoint_encoded_len(
        &self,
    ) -> Result<usize, VersionedTwoVcpuTwoDeviceTransactionError> {
        Ok(self.checkpoint.encode()?.len())
    }

    #[must_use]
    pub(crate) fn registration_pair_encoded_len(&self) -> usize {
        self.registrations.encode().len()
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, VersionedTwoVcpuTwoDeviceTransactionError> {
        let pair = self.registrations.materialize()?;
        validate_transaction_registration_binding(self.checkpoint.device_bars(), pair)?;
        let checkpoint = self.checkpoint.encode()?;
        let registrations = self.registrations.encode();
        let total_len = TWO_VCPU_TWO_DEVICE_TRANSACTION_HEADER_LEN
            .checked_add(checkpoint.len())
            .and_then(|length| length.checked_add(registrations.len()))
            .ok_or(VersionedTwoVcpuTwoDeviceTransactionError::LengthOverflow)?;

        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_MAGIC);
        bytes.extend_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_VERSION.to_le_bytes());
        bytes.extend_from_slice(
            &VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_ARCH_X86_64.to_le_bytes(),
        );
        bytes.extend_from_slice(&(TWO_VCPU_TWO_DEVICE_TRANSACTION_HEADER_LEN as u32).to_le_bytes());
        bytes.extend_from_slice(&(total_len as u64).to_le_bytes());
        bytes.extend_from_slice(&(checkpoint.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(registrations.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&checkpoint);
        bytes.extend_from_slice(&registrations);
        debug_assert_eq!(bytes.len(), total_len);
        Ok(bytes)
    }

    pub(crate) fn decode(
        bytes: &[u8],
    ) -> Result<Self, VersionedTwoVcpuTwoDeviceTransactionError> {
        if bytes.len() < TWO_VCPU_TWO_DEVICE_TRANSACTION_HEADER_LEN {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidTotalLength {
                declared: 0,
                actual: bytes.len(),
            });
        }
        if bytes[0..8] != VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_MAGIC {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version field"));
        if version != VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_VERSION {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::UnsupportedVersion(version));
        }
        let architecture =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed architecture field"));
        if architecture != VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_ARCH_X86_64 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::UnsupportedArchitecture(
                architecture,
            ));
        }
        let header_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header length field"));
        if header_len != TWO_VCPU_TWO_DEVICE_TRANSACTION_HEADER_LEN as u32 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidHeaderLength(
                header_len,
            ));
        }
        let declared_total =
            u64::from_le_bytes(bytes[16..24].try_into().expect("fixed total length field"));
        if usize::try_from(declared_total).ok() != Some(bytes.len()) {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidTotalLength {
                declared: declared_total,
                actual: bytes.len(),
            });
        }
        let checkpoint_len =
            u64::from_le_bytes(bytes[24..32].try_into().expect("fixed checkpoint length field"));
        let registration_len = u64::from_le_bytes(
            bytes[32..40]
                .try_into()
                .expect("fixed registration-pair length field"),
        );
        let flags = u32::from_le_bytes(bytes[40..44].try_into().expect("fixed flags field"));
        if flags != 0 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::NonZeroFlags(flags));
        }
        let reserved =
            u32::from_le_bytes(bytes[44..48].try_into().expect("fixed reserved field"));
        if reserved != 0 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::NonZeroReserved(
                reserved,
            ));
        }
        let checkpoint_len = usize::try_from(checkpoint_len).map_err(|_| {
            VersionedTwoVcpuTwoDeviceTransactionError::InvalidCheckpointLength(checkpoint_len)
        })?;
        if checkpoint_len == 0 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidCheckpointLength(0));
        }
        if registration_len != VERSIONED_HOST_REGISTRATION_PAIR_LEN as u64 {
            return Err(
                VersionedTwoVcpuTwoDeviceTransactionError::InvalidRegistrationPairLength(
                    registration_len,
                ),
            );
        }
        let registration_len = usize::try_from(registration_len).map_err(|_| {
            VersionedTwoVcpuTwoDeviceTransactionError::InvalidRegistrationPairLength(
                registration_len,
            )
        })?;
        let expected_len = TWO_VCPU_TWO_DEVICE_TRANSACTION_HEADER_LEN
            .checked_add(checkpoint_len)
            .and_then(|length| length.checked_add(registration_len))
            .ok_or(VersionedTwoVcpuTwoDeviceTransactionError::LengthOverflow)?;
        if expected_len != bytes.len() {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidTotalLength {
                declared: declared_total,
                actual: expected_len,
            });
        }
        let checkpoint_start = TWO_VCPU_TWO_DEVICE_TRANSACTION_HEADER_LEN;
        let checkpoint_end = checkpoint_start + checkpoint_len;
        let checkpoint =
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&bytes[checkpoint_start..checkpoint_end])?;
        let registrations =
            VersionedHostRegistrationPairV1::decode(&bytes[checkpoint_end..])?;
        let pair = registrations.materialize()?;
        validate_transaction_registration_binding(checkpoint.device_bars(), pair)?;
        Ok(Self {
            checkpoint,
            registrations,
        })
    }

    pub(crate) fn materialize(
        &self,
        host_msrs: &crate::kvm::msr::HostMsrIndexList,
    ) -> Result<
        (
            BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint,
            HostRegistrationSpecPair,
        ),
        VersionedTwoVcpuTwoDeviceTransactionError,
    > {
        let checkpoint = self.checkpoint.materialize(host_msrs)?;
        let pair = self.registrations.materialize()?;
        validate_transaction_registration_binding(checkpoint.device_bars(), pair)?;
        Ok((checkpoint, pair))
    }
}

const VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_V2: u16 = 2;
const TWO_VCPU_TWO_DEVICE_TRANSACTION_V2_HEADER_LEN: usize = 56;
const PENDING_COMPLETION_TOKEN_LEN: usize = 16;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VersionedTwoVcpuTwoDeviceTransactionV2 {
    checkpoint: VersionedTwoVcpuTwoDeviceCheckpointV1,
    registrations: VersionedHostRegistrationPairV1,
    pending_completion: VirtioBlkPendingCompletionToken,
}

impl VersionedTwoVcpuTwoDeviceTransactionV2 {
    pub(crate) fn from_checkpoint_pair_and_pending_completion(
        checkpoint: &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint,
        pair: HostRegistrationSpecPair,
        pending_completion: VirtioBlkPendingCompletionToken,
    ) -> Result<Self, VersionedTwoVcpuTwoDeviceTransactionError> {
        validate_transaction_registration_binding(checkpoint.device_bars(), pair)?;
        let versioned_checkpoint =
            VersionedTwoVcpuTwoDeviceCheckpointV1::from_checkpoint_with_pending_completion(
                checkpoint,
                &pending_completion,
            )?;
        versioned_checkpoint.validate_pending_completion_token(&pending_completion)?;
        Ok(Self {
            checkpoint: versioned_checkpoint,
            registrations: VersionedHostRegistrationPairV1::from_pair(pair),
            pending_completion,
        })
    }

    #[must_use]
    pub(crate) const fn version(&self) -> u16 {
        VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_V2
    }

    #[must_use]
    pub(crate) const fn bars(&self) -> [u64; 2] {
        self.checkpoint.device_bars()
    }

    #[must_use]
    pub(crate) fn page_count(&self) -> usize {
        self.checkpoint.page_count()
    }

    #[must_use]
    pub(crate) fn pending_completion(&self) -> &VirtioBlkPendingCompletionToken {
        &self.pending_completion
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, VersionedTwoVcpuTwoDeviceTransactionError> {
        let pair = self.registrations.materialize()?;
        validate_transaction_registration_binding(self.checkpoint.device_bars(), pair)?;
        self.checkpoint
            .validate_pending_completion_token(&self.pending_completion)?;
        let checkpoint = self.checkpoint.encode()?;
        let registrations = self.registrations.encode();
        let token = encode_pending_completion_token(&self.pending_completion);
        let total_len = TWO_VCPU_TWO_DEVICE_TRANSACTION_V2_HEADER_LEN
            .checked_add(checkpoint.len())
            .and_then(|length| length.checked_add(registrations.len()))
            .and_then(|length| length.checked_add(token.len()))
            .ok_or(VersionedTwoVcpuTwoDeviceTransactionError::LengthOverflow)?;

        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_MAGIC);
        bytes.extend_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_V2.to_le_bytes());
        bytes.extend_from_slice(
            &VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_ARCH_X86_64.to_le_bytes(),
        );
        bytes.extend_from_slice(
            &(TWO_VCPU_TWO_DEVICE_TRANSACTION_V2_HEADER_LEN as u32).to_le_bytes(),
        );
        bytes.extend_from_slice(&(total_len as u64).to_le_bytes());
        bytes.extend_from_slice(&(checkpoint.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(registrations.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(token.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&checkpoint);
        bytes.extend_from_slice(&registrations);
        bytes.extend_from_slice(&token);
        debug_assert_eq!(bytes.len(), total_len);
        Ok(bytes)
    }

    pub(crate) fn decode(
        bytes: &[u8],
    ) -> Result<Self, VersionedTwoVcpuTwoDeviceTransactionError> {
        if bytes.len() < TWO_VCPU_TWO_DEVICE_TRANSACTION_V2_HEADER_LEN {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidTotalLength {
                declared: 0,
                actual: bytes.len(),
            });
        }
        if bytes[0..8] != VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_MAGIC {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version field"));
        if version != VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_V2 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::UnsupportedVersion(version));
        }
        let architecture =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed architecture field"));
        if architecture != VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_ARCH_X86_64 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::UnsupportedArchitecture(
                architecture,
            ));
        }
        let header_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header length field"));
        if header_len != TWO_VCPU_TWO_DEVICE_TRANSACTION_V2_HEADER_LEN as u32 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidHeaderLength(
                header_len,
            ));
        }
        let declared_total =
            u64::from_le_bytes(bytes[16..24].try_into().expect("fixed total length field"));
        if usize::try_from(declared_total).ok() != Some(bytes.len()) {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidTotalLength {
                declared: declared_total,
                actual: bytes.len(),
            });
        }
        let checkpoint_len =
            u64::from_le_bytes(bytes[24..32].try_into().expect("fixed checkpoint length field"));
        let registration_len = u64::from_le_bytes(
            bytes[32..40]
                .try_into()
                .expect("fixed registration-pair length field"),
        );
        let token_len =
            u64::from_le_bytes(bytes[40..48].try_into().expect("fixed token length field"));
        if token_len != PENDING_COMPLETION_TOKEN_LEN as u64 {
            return Err(
                VersionedTwoVcpuTwoDeviceTransactionError::InvalidPendingCompletionTokenLength(
                    token_len,
                ),
            );
        }
        let flags = u32::from_le_bytes(bytes[48..52].try_into().expect("fixed flags field"));
        if flags != 0 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::NonZeroFlags(flags));
        }
        let reserved =
            u32::from_le_bytes(bytes[52..56].try_into().expect("fixed reserved field"));
        if reserved != 0 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::NonZeroReserved(
                reserved,
            ));
        }
        let checkpoint_len = usize::try_from(checkpoint_len).map_err(|_| {
            VersionedTwoVcpuTwoDeviceTransactionError::InvalidCheckpointLength(checkpoint_len)
        })?;
        if checkpoint_len == 0 {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidCheckpointLength(0));
        }
        if registration_len != VERSIONED_HOST_REGISTRATION_PAIR_LEN as u64 {
            return Err(
                VersionedTwoVcpuTwoDeviceTransactionError::InvalidRegistrationPairLength(
                    registration_len,
                ),
            );
        }
        let registration_len = VERSIONED_HOST_REGISTRATION_PAIR_LEN;
        let expected_len = TWO_VCPU_TWO_DEVICE_TRANSACTION_V2_HEADER_LEN
            .checked_add(checkpoint_len)
            .and_then(|length| length.checked_add(registration_len))
            .and_then(|length| length.checked_add(PENDING_COMPLETION_TOKEN_LEN))
            .ok_or(VersionedTwoVcpuTwoDeviceTransactionError::LengthOverflow)?;
        if expected_len != bytes.len() {
            return Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidTotalLength {
                declared: declared_total,
                actual: expected_len,
            });
        }

        let checkpoint_start = TWO_VCPU_TWO_DEVICE_TRANSACTION_V2_HEADER_LEN;
        let checkpoint_end = checkpoint_start + checkpoint_len;
        let registrations_end = checkpoint_end + registration_len;
        let checkpoint =
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&bytes[checkpoint_start..checkpoint_end])?;
        let registrations =
            VersionedHostRegistrationPairV1::decode(&bytes[checkpoint_end..registrations_end])?;
        let pending_completion =
            decode_pending_completion_token(&bytes[registrations_end..])?;
        let pair = registrations.materialize()?;
        validate_transaction_registration_binding(checkpoint.device_bars(), pair)?;
        checkpoint.validate_pending_completion_token(&pending_completion)?;
        Ok(Self {
            checkpoint,
            registrations,
            pending_completion,
        })
    }

    pub(crate) fn materialize(
        self,
        host_msrs: &crate::kvm::msr::HostMsrIndexList,
    ) -> Result<
        (
            BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpoint,
            HostRegistrationSpecPair,
            VirtioBlkPendingCompletionToken,
        ),
        VersionedTwoVcpuTwoDeviceTransactionError,
    > {
        let checkpoint = self.checkpoint.materialize(host_msrs)?;
        let pair = self.registrations.materialize()?;
        validate_transaction_registration_binding(checkpoint.device_bars(), pair)?;
        self.pending_completion
            .validate_device(
                checkpoint
                    .device(self.pending_completion.bar0())
                    .ok_or_else(|| {
                        VersionedTwoVcpuTwoDeviceTransactionError::PendingCompletion(format!(
                            "BAR {:#x} disappeared during materialization",
                            self.pending_completion.bar0()
                        ))
                    })?,
            )
            .map_err(|error| {
                VersionedTwoVcpuTwoDeviceTransactionError::PendingCompletion(error.to_string())
            })?;
        Ok((checkpoint, pair, self.pending_completion))
    }
}

fn encode_pending_completion_token(token: &VirtioBlkPendingCompletionToken) -> [u8; PENDING_COMPLETION_TOKEN_LEN] {
    let mut bytes = [0_u8; PENDING_COMPLETION_TOKEN_LEN];
    bytes[0..8].copy_from_slice(&token.bar0().to_le_bytes());
    bytes[8..10].copy_from_slice(&token.queue().to_le_bytes());
    bytes[10..12].copy_from_slice(&token.last_avail_idx().to_le_bytes());
    bytes[12..14].copy_from_slice(&token.last_used_idx().to_le_bytes());
    bytes
}

fn decode_pending_completion_token(
    bytes: &[u8],
) -> Result<VirtioBlkPendingCompletionToken, VersionedTwoVcpuTwoDeviceTransactionError> {
    if bytes.len() != PENDING_COMPLETION_TOKEN_LEN {
        return Err(
            VersionedTwoVcpuTwoDeviceTransactionError::InvalidPendingCompletionTokenLength(
                bytes.len() as u64,
            ),
        );
    }
    let reserved = u16::from_le_bytes(bytes[14..16].try_into().expect("fixed reserved field"));
    if reserved != 0 {
        return Err(VersionedTwoVcpuTwoDeviceTransactionError::PendingCompletion(
            format!("token reserved field must be zero, got {reserved:#x}"),
        ));
    }
    VirtioBlkPendingCompletionToken::from_parts(
        u64::from_le_bytes(bytes[0..8].try_into().expect("fixed token BAR")),
        u16::from_le_bytes(bytes[8..10].try_into().expect("fixed token queue")),
        u16::from_le_bytes(bytes[10..12].try_into().expect("fixed token avail index")),
        u16::from_le_bytes(bytes[12..14].try_into().expect("fixed token used index")),
    )
    .map_err(|error| VersionedTwoVcpuTwoDeviceTransactionError::PendingCompletion(error.to_string()))
}

fn validate_transaction_registration_binding(
    bars: [u64; 2],
    pair: HostRegistrationSpecPair,
) -> Result<(), VersionedTwoVcpuTwoDeviceTransactionError> {
    require_registration_pair_matches_devices(pair, bars).map_err(|error| {
        VersionedTwoVcpuTwoDeviceTransactionError::RegistrationBinding(error.to_string())
    })
}

#[cfg(test)]
mod versioned_two_vcpu_two_device_transaction_schema_tests {
    use super::*;
    use crate::kvm::sys::{
        default_two_host_registration_pair, HostRegistrationSpec,
        TWO_HOST_REGISTRATION_FIRST_BAR, TWO_HOST_REGISTRATION_SECOND_BAR,
    };

    fn two_device_checkpoint_header_fixture() -> Vec<u8> {
        let controller_len = 1_usize;
        let total_len = TWO_VCPU_TWO_DEVICE_CHECKPOINT_HEADER_LEN
            + controller_len
            + 2 * VIRTIO_BLK_STATE_LEN;
        let mut bytes = vec![0_u8; total_len];
        bytes[0..8].copy_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_MAGIC);
        bytes[8..10]
            .copy_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_VERSION.to_le_bytes());
        bytes[10..12]
            .copy_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_CHECKPOINT_ARCH_X86_64.to_le_bytes());
        bytes[12..16]
            .copy_from_slice(&(TWO_VCPU_TWO_DEVICE_CHECKPOINT_HEADER_LEN as u32).to_le_bytes());
        bytes[16..24].copy_from_slice(&(total_len as u64).to_le_bytes());
        bytes[24..32].copy_from_slice(&(controller_len as u64).to_le_bytes());
        bytes[32..40].copy_from_slice(&(VIRTIO_BLK_STATE_LEN as u64).to_le_bytes());
        bytes[40..44].copy_from_slice(&TWO_VCPU_TWO_DEVICE_CHECKPOINT_DEVICE_COUNT.to_le_bytes());
        bytes
    }

    #[test]
    fn two_device_checkpoint_header_fails_closed_before_nested_decode() {
        let base = two_device_checkpoint_header_fixture();

        let mut bad_magic = base.clone();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&bad_magic),
            Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidMagic)
        );

        let mut bad_version = base.clone();
        bad_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&bad_version),
            Err(VersionedTwoVcpuTwoDeviceCheckpointError::UnsupportedVersion(2))
        );

        let mut bad_architecture = base.clone();
        bad_architecture[10..12].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&bad_architecture),
            Err(
                VersionedTwoVcpuTwoDeviceCheckpointError::UnsupportedArchitecture(2)
            )
        );

        let mut bad_header_len = base.clone();
        bad_header_len[12..16].copy_from_slice(&55_u32.to_le_bytes());
        assert_eq!(
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&bad_header_len),
            Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidHeaderLength(55))
        );

        let mut bad_device_len = base.clone();
        bad_device_len[32..40]
            .copy_from_slice(&((VIRTIO_BLK_STATE_LEN as u64) - 1).to_le_bytes());
        assert_eq!(
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&bad_device_len),
            Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidDeviceLength(
                (VIRTIO_BLK_STATE_LEN as u64) - 1
            ))
        );

        let mut bad_count = base.clone();
        bad_count[40..44].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&bad_count),
            Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidDeviceCount(1))
        );

        let mut bad_flags = base.clone();
        bad_flags[44..48].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&bad_flags),
            Err(VersionedTwoVcpuTwoDeviceCheckpointError::NonZeroFlags(1))
        );

        let mut bad_reserved = base.clone();
        bad_reserved[48..56].copy_from_slice(&1_u64.to_le_bytes());
        assert_eq!(
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&bad_reserved),
            Err(VersionedTwoVcpuTwoDeviceCheckpointError::NonZeroReserved(1))
        );

        let mut truncated = base;
        truncated.pop();
        assert!(matches!(
            VersionedTwoVcpuTwoDeviceCheckpointV1::decode(&truncated),
            Err(VersionedTwoVcpuTwoDeviceCheckpointError::InvalidTotalLength { .. })
        ));
    }

    #[test]
    fn two_device_checkpoint_bars_must_remain_canonical() {
        assert!(validate_versioned_two_device_bars([0x1000_0000, 0x1000_1000]).is_ok());
        assert_eq!(
            validate_versioned_two_device_bars([0x1000_1000, 0x1000_0000]),
            Err(VersionedTwoVcpuTwoDeviceCheckpointError::NonCanonicalBars {
                first: 0x1000_1000,
                second: 0x1000_0000,
            })
        );
        assert!(matches!(
            validate_versioned_two_device_bars([0x1000_0001, 0x1000_1000]),
            Err(VersionedTwoVcpuTwoDeviceCheckpointError::NonCanonicalBars { .. })
        ));
    }

    #[test]
    fn registration_binding_is_revalidated_at_the_wire_boundary() {
        let bars = [
            TWO_HOST_REGISTRATION_FIRST_BAR,
            TWO_HOST_REGISTRATION_SECOND_BAR,
        ];
        validate_transaction_registration_binding(
            bars,
            default_two_host_registration_pair().unwrap(),
        )
        .unwrap();

        let wrong = HostRegistrationSpecPair::new([
            HostRegistrationSpec::new(bars[0] + 0x200, 2, 0, 0).unwrap(),
            HostRegistrationSpec::new(bars[1] + 0x200, 2, 0, 1).unwrap(),
        ])
        .unwrap();
        assert!(matches!(
            validate_transaction_registration_binding(bars, wrong),
            Err(VersionedTwoVcpuTwoDeviceTransactionError::RegistrationBinding(_))
        ));
    }

    #[test]
    fn pending_completion_token_wire_is_fixed_and_fails_closed() {
        let token = VirtioBlkPendingCompletionToken::from_parts(
            TWO_HOST_REGISTRATION_FIRST_BAR,
            0,
            1,
            1,
        )
        .unwrap();
        let bytes = encode_pending_completion_token(&token);
        assert_eq!(bytes.len(), PENDING_COMPLETION_TOKEN_LEN);
        assert_eq!(decode_pending_completion_token(&bytes).unwrap(), token);

        let mut bad_reserved = bytes;
        bad_reserved[14..16].copy_from_slice(&1_u16.to_le_bytes());
        assert!(matches!(
            decode_pending_completion_token(&bad_reserved),
            Err(VersionedTwoVcpuTwoDeviceTransactionError::PendingCompletion(_))
        ));
        assert!(decode_pending_completion_token(&bytes[..15]).is_err());
        assert!(VirtioBlkPendingCompletionToken::from_parts(
            TWO_HOST_REGISTRATION_FIRST_BAR,
            0,
            1,
            2,
        )
        .is_err());
    }

    #[test]
    fn outer_transaction_header_fails_closed_before_nested_decode() {
        let checkpoint_len = 1_usize;
        let registration_len = VERSIONED_HOST_REGISTRATION_PAIR_LEN;
        let total_len =
            TWO_VCPU_TWO_DEVICE_TRANSACTION_HEADER_LEN + checkpoint_len + registration_len;
        let mut base = vec![0_u8; total_len];
        base[0..8].copy_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_MAGIC);
        base[8..10].copy_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_VERSION.to_le_bytes());
        base[10..12]
            .copy_from_slice(&VERSIONED_TWO_VCPU_TWO_DEVICE_TRANSACTION_ARCH_X86_64.to_le_bytes());
        base[12..16]
            .copy_from_slice(&(TWO_VCPU_TWO_DEVICE_TRANSACTION_HEADER_LEN as u32).to_le_bytes());
        base[16..24].copy_from_slice(&(total_len as u64).to_le_bytes());
        base[24..32].copy_from_slice(&(checkpoint_len as u64).to_le_bytes());
        base[32..40].copy_from_slice(&(registration_len as u64).to_le_bytes());

        let mut bad_magic = base.clone();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            VersionedTwoVcpuTwoDeviceTransactionV1::decode(&bad_magic),
            Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidMagic)
        );

        let mut bad_flags = base.clone();
        bad_flags[40..44].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedTwoVcpuTwoDeviceTransactionV1::decode(&bad_flags),
            Err(VersionedTwoVcpuTwoDeviceTransactionError::NonZeroFlags(1))
        );

        let mut truncated = base;
        truncated.pop();
        assert!(matches!(
            VersionedTwoVcpuTwoDeviceTransactionV1::decode(&truncated),
            Err(VersionedTwoVcpuTwoDeviceTransactionError::InvalidTotalLength { .. })
        ));
    }
}
