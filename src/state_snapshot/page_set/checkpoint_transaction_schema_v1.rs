use crate::kvm::msr::HostMsrIndexList;
use crate::kvm::sys::{
    HostRegistrationSpec, VersionedHostRegistrationSpecError, VersionedHostRegistrationSpecV1,
};
use std::fmt;

pub const VERSIONED_CHECKPOINT_TRANSACTION_MAGIC: [u8; 8] = *b"MHVTXN\0\0";
pub const VERSIONED_CHECKPOINT_TRANSACTION_VERSION: u16 = 1;
pub const VERSIONED_CHECKPOINT_TRANSACTION_ARCH_X86_64: u16 = 1;
const CHECKPOINT_TRANSACTION_HEADER_LEN: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VersionedCheckpointTransactionError {
    InvalidMagic,
    UnsupportedVersion(u16),
    UnsupportedArchitecture(u16),
    InvalidHeaderLength(u32),
    InvalidTotalLength { declared: u64, actual: usize },
    InvalidCheckpointLength(u64),
    InvalidRegistrationLength(u64),
    NonZeroFlags(u32),
    NonZeroReserved(u32),
    LengthOverflow,
    Checkpoint(VersionedFullControllerVirtioBlkCheckpointError),
    Registration(VersionedHostRegistrationSpecError),
}

impl fmt::Display for VersionedCheckpointTransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "checkpoint transaction magic does not match MHVTXN"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported checkpoint transaction version {version}")
            }
            Self::UnsupportedArchitecture(architecture) => write!(
                f,
                "unsupported checkpoint transaction architecture identifier {architecture}"
            ),
            Self::InvalidHeaderLength(length) => write!(
                f,
                "checkpoint transaction header length {length} is not {CHECKPOINT_TRANSACTION_HEADER_LEN}"
            ),
            Self::InvalidTotalLength { declared, actual } => write!(
                f,
                "checkpoint transaction declares total length {declared}, actual byte length is {actual}"
            ),
            Self::InvalidCheckpointLength(length) => write!(
                f,
                "checkpoint transaction nested checkpoint length {length} is invalid"
            ),
            Self::InvalidRegistrationLength(length) => write!(
                f,
                "checkpoint transaction nested registration length {length} is invalid"
            ),
            Self::NonZeroFlags(flags) => write!(
                f,
                "checkpoint transaction v1 flags must be zero, got {flags:#x}"
            ),
            Self::NonZeroReserved(value) => write!(
                f,
                "checkpoint transaction reserved field must be zero, got {value:#x}"
            ),
            Self::LengthOverflow => write!(f, "checkpoint transaction length arithmetic overflowed"),
            Self::Checkpoint(error) => write!(f, "nested checkpoint schema is invalid: {error}"),
            Self::Registration(error) => {
                write!(f, "nested host-registration schema is invalid: {error}")
            }
        }
    }
}

impl std::error::Error for VersionedCheckpointTransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Checkpoint(error) => Some(error),
            Self::Registration(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VersionedFullControllerVirtioBlkCheckpointError>
    for VersionedCheckpointTransactionError
{
    fn from(error: VersionedFullControllerVirtioBlkCheckpointError) -> Self {
        Self::Checkpoint(error)
    }
}

impl From<VersionedHostRegistrationSpecError> for VersionedCheckpointTransactionError {
    fn from(error: VersionedHostRegistrationSpecError) -> Self {
        Self::Registration(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionedCheckpointTransactionV1 {
    checkpoint: VersionedFullControllerVirtioBlkCheckpointV1,
    registration: VersionedHostRegistrationSpecV1,
}

impl VersionedCheckpointTransactionV1 {
    pub(crate) fn from_checkpoint_and_spec(
        checkpoint: &BoundedFullControllerVirtioBlkCheckpoint,
        spec: HostRegistrationSpec,
    ) -> Result<Self, VersionedCheckpointTransactionError> {
        Ok(Self {
            checkpoint: VersionedFullControllerVirtioBlkCheckpointV1::from_checkpoint(checkpoint)?,
            registration: VersionedHostRegistrationSpecV1::from_spec(spec),
        })
    }

    #[must_use]
    pub(crate) const fn version(&self) -> u16 {
        VERSIONED_CHECKPOINT_TRANSACTION_VERSION
    }

    #[must_use]
    pub(crate) const fn checkpoint_version(&self) -> u16 {
        self.checkpoint.version()
    }

    #[must_use]
    pub(crate) const fn registration_version(&self) -> u16 {
        self.registration.version()
    }

    pub(crate) fn checkpoint_encoded_len(
        &self,
    ) -> Result<usize, VersionedCheckpointTransactionError> {
        Ok(self.checkpoint.encode()?.len())
    }

    #[must_use]
    pub(crate) fn registration_encoded_len(&self) -> usize {
        self.registration.encode().len()
    }

    #[must_use]
    pub(crate) fn checkpoint_page_count(&self) -> usize {
        self.checkpoint.page_count()
    }

    #[must_use]
    pub(crate) fn checkpoint_msr_count(&self) -> usize {
        self.checkpoint.msr_count()
    }

    #[must_use]
    pub(crate) const fn checkpoint_bar0(&self) -> u64 {
        self.checkpoint.bar0()
    }

    #[must_use]
    pub(crate) const fn checkpoint_backing_len(&self) -> usize {
        self.checkpoint.backing_len()
    }

    #[must_use]
    pub(crate) fn registration_spec(&self) -> HostRegistrationSpec {
        self.registration.spec()
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, VersionedCheckpointTransactionError> {
        let checkpoint = self.checkpoint.encode()?;
        let registration = self.registration.encode();
        let total_len = CHECKPOINT_TRANSACTION_HEADER_LEN
            .checked_add(checkpoint.len())
            .and_then(|length| length.checked_add(registration.len()))
            .ok_or(VersionedCheckpointTransactionError::LengthOverflow)?;

        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_CHECKPOINT_TRANSACTION_MAGIC);
        bytes.extend_from_slice(&VERSIONED_CHECKPOINT_TRANSACTION_VERSION.to_le_bytes());
        bytes.extend_from_slice(&VERSIONED_CHECKPOINT_TRANSACTION_ARCH_X86_64.to_le_bytes());
        bytes.extend_from_slice(&(CHECKPOINT_TRANSACTION_HEADER_LEN as u32).to_le_bytes());
        bytes.extend_from_slice(&(total_len as u64).to_le_bytes());
        bytes.extend_from_slice(&(checkpoint.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(registration.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&checkpoint);
        bytes.extend_from_slice(&registration);
        debug_assert_eq!(bytes.len(), total_len);
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, VersionedCheckpointTransactionError> {
        if bytes.len() < CHECKPOINT_TRANSACTION_HEADER_LEN {
            return Err(VersionedCheckpointTransactionError::InvalidTotalLength {
                declared: 0,
                actual: bytes.len(),
            });
        }
        if bytes[0..8] != VERSIONED_CHECKPOINT_TRANSACTION_MAGIC {
            return Err(VersionedCheckpointTransactionError::InvalidMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version field"));
        if version != VERSIONED_CHECKPOINT_TRANSACTION_VERSION {
            return Err(VersionedCheckpointTransactionError::UnsupportedVersion(version));
        }
        let architecture =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed architecture field"));
        if architecture != VERSIONED_CHECKPOINT_TRANSACTION_ARCH_X86_64 {
            return Err(VersionedCheckpointTransactionError::UnsupportedArchitecture(
                architecture,
            ));
        }
        let header_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header-length field"));
        if header_len as usize != CHECKPOINT_TRANSACTION_HEADER_LEN {
            return Err(VersionedCheckpointTransactionError::InvalidHeaderLength(
                header_len,
            ));
        }
        let declared_total =
            u64::from_le_bytes(bytes[16..24].try_into().expect("fixed total-length field"));
        if usize::try_from(declared_total).ok() != Some(bytes.len()) {
            return Err(VersionedCheckpointTransactionError::InvalidTotalLength {
                declared: declared_total,
                actual: bytes.len(),
            });
        }
        let checkpoint_len =
            u64::from_le_bytes(bytes[24..32].try_into().expect("fixed checkpoint-length field"));
        let registration_len =
            u64::from_le_bytes(bytes[32..40].try_into().expect("fixed registration-length field"));
        let flags = u32::from_le_bytes(bytes[40..44].try_into().expect("fixed flags field"));
        if flags != 0 {
            return Err(VersionedCheckpointTransactionError::NonZeroFlags(flags));
        }
        let reserved =
            u32::from_le_bytes(bytes[44..48].try_into().expect("fixed reserved field"));
        if reserved != 0 {
            return Err(VersionedCheckpointTransactionError::NonZeroReserved(
                reserved,
            ));
        }

        let checkpoint_len = usize::try_from(checkpoint_len)
            .map_err(|_| VersionedCheckpointTransactionError::InvalidCheckpointLength(checkpoint_len))?;
        let registration_len = usize::try_from(registration_len)
            .map_err(|_| VersionedCheckpointTransactionError::InvalidRegistrationLength(registration_len))?;
        if checkpoint_len == 0 {
            return Err(VersionedCheckpointTransactionError::InvalidCheckpointLength(0));
        }
        if registration_len == 0 {
            return Err(VersionedCheckpointTransactionError::InvalidRegistrationLength(0));
        }
        let expected_len = CHECKPOINT_TRANSACTION_HEADER_LEN
            .checked_add(checkpoint_len)
            .and_then(|length| length.checked_add(registration_len))
            .ok_or(VersionedCheckpointTransactionError::LengthOverflow)?;
        if expected_len != bytes.len() {
            return Err(VersionedCheckpointTransactionError::InvalidTotalLength {
                declared: declared_total,
                actual: expected_len,
            });
        }

        let checkpoint_start = CHECKPOINT_TRANSACTION_HEADER_LEN;
        let checkpoint_end = checkpoint_start + checkpoint_len;
        Ok(Self {
            checkpoint: VersionedFullControllerVirtioBlkCheckpointV1::decode(
                &bytes[checkpoint_start..checkpoint_end],
            )?,
            registration: VersionedHostRegistrationSpecV1::decode(&bytes[checkpoint_end..])?,
        })
    }

    pub(crate) fn materialize(
        &self,
        host_msrs: &HostMsrIndexList,
    ) -> Result<
        (
            BoundedFullControllerVirtioBlkCheckpoint,
            HostRegistrationSpec,
        ),
        VersionedCheckpointTransactionError,
    > {
        Ok((
            self.checkpoint.materialize(host_msrs)?,
            self.registration.spec(),
        ))
    }
}

#[cfg(test)]
mod versioned_checkpoint_transaction_tests {
    use super::*;

    fn minimally_sized_outer_envelope() -> Vec<u8> {
        let checkpoint_len = 1_usize;
        let registration_len = 1_usize;
        let total_len =
            CHECKPOINT_TRANSACTION_HEADER_LEN + checkpoint_len + registration_len;
        let mut bytes = vec![0_u8; total_len];
        bytes[0..8].copy_from_slice(&VERSIONED_CHECKPOINT_TRANSACTION_MAGIC);
        bytes[8..10].copy_from_slice(&VERSIONED_CHECKPOINT_TRANSACTION_VERSION.to_le_bytes());
        bytes[10..12].copy_from_slice(&VERSIONED_CHECKPOINT_TRANSACTION_ARCH_X86_64.to_le_bytes());
        bytes[12..16].copy_from_slice(&(CHECKPOINT_TRANSACTION_HEADER_LEN as u32).to_le_bytes());
        bytes[16..24].copy_from_slice(&(total_len as u64).to_le_bytes());
        bytes[24..32].copy_from_slice(&(checkpoint_len as u64).to_le_bytes());
        bytes[32..40].copy_from_slice(&(registration_len as u64).to_le_bytes());
        bytes
    }

    #[test]
    fn outer_header_fails_closed_before_nested_decode() {
        let base = minimally_sized_outer_envelope();

        let mut bad_magic = base.clone();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            VersionedCheckpointTransactionV1::decode(&bad_magic),
            Err(VersionedCheckpointTransactionError::InvalidMagic)
        );

        let mut bad_version = base.clone();
        bad_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedCheckpointTransactionV1::decode(&bad_version),
            Err(VersionedCheckpointTransactionError::UnsupportedVersion(2))
        );

        let mut bad_arch = base.clone();
        bad_arch[10..12].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedCheckpointTransactionV1::decode(&bad_arch),
            Err(VersionedCheckpointTransactionError::UnsupportedArchitecture(2))
        );

        let mut bad_header = base.clone();
        bad_header[12..16].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            VersionedCheckpointTransactionV1::decode(&bad_header),
            Err(VersionedCheckpointTransactionError::InvalidHeaderLength(0))
        );

        let mut bad_total = base.clone();
        bad_total[16..24].copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            VersionedCheckpointTransactionV1::decode(&bad_total),
            Err(VersionedCheckpointTransactionError::InvalidTotalLength {
                declared: 0,
                actual: base.len(),
            })
        );

        let mut bad_checkpoint_len = base.clone();
        bad_checkpoint_len[24..32].copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            VersionedCheckpointTransactionV1::decode(&bad_checkpoint_len),
            Err(VersionedCheckpointTransactionError::InvalidCheckpointLength(0))
        );

        let mut bad_registration_len = base.clone();
        bad_registration_len[32..40].copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            VersionedCheckpointTransactionV1::decode(&bad_registration_len),
            Err(VersionedCheckpointTransactionError::InvalidRegistrationLength(0))
        );

        let mut bad_flags = base.clone();
        bad_flags[40..44].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedCheckpointTransactionV1::decode(&bad_flags),
            Err(VersionedCheckpointTransactionError::NonZeroFlags(1))
        );

        let mut bad_reserved = base.clone();
        bad_reserved[44..48].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedCheckpointTransactionV1::decode(&bad_reserved),
            Err(VersionedCheckpointTransactionError::NonZeroReserved(1))
        );

        let mut truncated = base;
        truncated.pop();
        assert!(matches!(
            VersionedCheckpointTransactionV1::decode(&truncated),
            Err(VersionedCheckpointTransactionError::InvalidTotalLength { .. })
        ));
    }
}
