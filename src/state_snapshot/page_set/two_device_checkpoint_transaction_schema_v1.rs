pub const VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_MAGIC: [u8; 8] = *b"MHVTX2\0\0";
pub const VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_VERSION: u16 = 1;
pub const VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_ARCH_X86_64: u16 = 1;
const TWO_DEVICE_CHECKPOINT_TRANSACTION_HEADER_LEN: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VersionedTwoDeviceCheckpointTransactionError {
    InvalidMagic,
    UnsupportedVersion(u16),
    UnsupportedArchitecture(u16),
    InvalidHeaderLength(u32),
    InvalidTotalLength { declared: u64, actual: usize },
    InvalidCheckpointLength(u64),
    InvalidRegistrationPairLength(u64),
    NonZeroFlags(u32),
    NonZeroReserved(u32),
    LengthOverflow,
    Checkpoint(VersionedFullControllerTwoVirtioBlkCheckpointError),
    RegistrationPair(crate::kvm::sys::VersionedHostRegistrationPairError),
}

impl std::fmt::Display for VersionedTwoDeviceCheckpointTransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic => write!(
                f,
                "two-device checkpoint transaction magic does not match MHVTX2"
            ),
            Self::UnsupportedVersion(version) => write!(
                f,
                "unsupported two-device checkpoint transaction version {version}"
            ),
            Self::UnsupportedArchitecture(architecture) => write!(
                f,
                "unsupported two-device checkpoint transaction architecture identifier {architecture}"
            ),
            Self::InvalidHeaderLength(length) => write!(
                f,
                "two-device checkpoint transaction header length {length} is not {TWO_DEVICE_CHECKPOINT_TRANSACTION_HEADER_LEN}"
            ),
            Self::InvalidTotalLength { declared, actual } => write!(
                f,
                "two-device checkpoint transaction declares total length {declared}, actual byte length is {actual}"
            ),
            Self::InvalidCheckpointLength(length) => write!(
                f,
                "two-device checkpoint transaction nested checkpoint length {length} is invalid"
            ),
            Self::InvalidRegistrationPairLength(length) => write!(
                f,
                "two-device checkpoint transaction registration-pair length {length} is not {}",
                crate::kvm::sys::VERSIONED_HOST_REGISTRATION_PAIR_LEN
            ),
            Self::NonZeroFlags(flags) => write!(
                f,
                "two-device checkpoint transaction v1 flags must be zero, got {flags:#x}"
            ),
            Self::NonZeroReserved(value) => write!(
                f,
                "two-device checkpoint transaction reserved field must be zero, got {value:#x}"
            ),
            Self::LengthOverflow => {
                write!(f, "two-device checkpoint transaction length arithmetic overflowed")
            }
            Self::Checkpoint(error) => {
                write!(f, "nested two-device checkpoint schema is invalid: {error}")
            }
            Self::RegistrationPair(error) => {
                write!(f, "nested registration-pair schema is invalid: {error}")
            }
        }
    }
}

impl std::error::Error for VersionedTwoDeviceCheckpointTransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Checkpoint(error) => Some(error),
            Self::RegistrationPair(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VersionedFullControllerTwoVirtioBlkCheckpointError>
    for VersionedTwoDeviceCheckpointTransactionError
{
    fn from(error: VersionedFullControllerTwoVirtioBlkCheckpointError) -> Self {
        Self::Checkpoint(error)
    }
}

impl From<crate::kvm::sys::VersionedHostRegistrationPairError>
    for VersionedTwoDeviceCheckpointTransactionError
{
    fn from(error: crate::kvm::sys::VersionedHostRegistrationPairError) -> Self {
        Self::RegistrationPair(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionedTwoDeviceCheckpointTransactionV1 {
    checkpoint: VersionedFullControllerTwoVirtioBlkCheckpointV1,
    registrations: crate::kvm::sys::VersionedHostRegistrationPairV1,
}

impl VersionedTwoDeviceCheckpointTransactionV1 {
    pub(crate) fn from_checkpoint_and_pair(
        checkpoint: &BoundedFullControllerTwoVirtioBlkCheckpoint,
        pair: crate::kvm::sys::HostRegistrationSpecPair,
    ) -> Result<Self, VersionedTwoDeviceCheckpointTransactionError> {
        Ok(Self {
            checkpoint: VersionedFullControllerTwoVirtioBlkCheckpointV1::from_checkpoint(
                checkpoint,
            )?,
            registrations: crate::kvm::sys::VersionedHostRegistrationPairV1::from_pair(pair),
        })
    }

    #[must_use]
    pub(crate) const fn version(&self) -> u16 {
        VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_VERSION
    }

    #[must_use]
    pub(crate) const fn checkpoint_version(&self) -> u16 {
        self.checkpoint.version()
    }

    #[must_use]
    pub(crate) const fn registration_pair_version(&self) -> u16 {
        self.registrations.version()
    }

    #[must_use]
    pub(crate) const fn registration_versions(&self) -> [u16; 2] {
        self.registrations.registration_versions()
    }

    pub(crate) fn checkpoint_encoded_len(
        &self,
    ) -> Result<usize, VersionedTwoDeviceCheckpointTransactionError> {
        Ok(self.checkpoint.encode()?.len())
    }

    #[must_use]
    pub(crate) fn registration_pair_encoded_len(&self) -> usize {
        self.registrations.encode().len()
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
    pub(crate) const fn checkpoint_bars(&self) -> [u64; 2] {
        self.checkpoint.device_bars()
    }

    #[must_use]
    pub(crate) const fn checkpoint_backing_len_each(&self) -> usize {
        self.checkpoint.backing_len_each()
    }

    pub(crate) fn encode(
        &self,
    ) -> Result<Vec<u8>, VersionedTwoDeviceCheckpointTransactionError> {
        let checkpoint = self.checkpoint.encode()?;
        let registrations = self.registrations.encode();
        let total_len = TWO_DEVICE_CHECKPOINT_TRANSACTION_HEADER_LEN
            .checked_add(checkpoint.len())
            .and_then(|length| length.checked_add(registrations.len()))
            .ok_or(VersionedTwoDeviceCheckpointTransactionError::LengthOverflow)?;

        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_MAGIC);
        bytes.extend_from_slice(&VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_VERSION.to_le_bytes());
        bytes.extend_from_slice(
            &VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_ARCH_X86_64.to_le_bytes(),
        );
        bytes.extend_from_slice(
            &(TWO_DEVICE_CHECKPOINT_TRANSACTION_HEADER_LEN as u32).to_le_bytes(),
        );
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
    ) -> Result<Self, VersionedTwoDeviceCheckpointTransactionError> {
        if bytes.len() < TWO_DEVICE_CHECKPOINT_TRANSACTION_HEADER_LEN {
            return Err(
                VersionedTwoDeviceCheckpointTransactionError::InvalidTotalLength {
                    declared: 0,
                    actual: bytes.len(),
                },
            );
        }
        if bytes[0..8] != VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_MAGIC {
            return Err(VersionedTwoDeviceCheckpointTransactionError::InvalidMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version field"));
        if version != VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_VERSION {
            return Err(
                VersionedTwoDeviceCheckpointTransactionError::UnsupportedVersion(version),
            );
        }
        let architecture =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed architecture field"));
        if architecture != VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_ARCH_X86_64 {
            return Err(
                VersionedTwoDeviceCheckpointTransactionError::UnsupportedArchitecture(
                    architecture,
                ),
            );
        }
        let header_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header-length field"));
        if header_len as usize != TWO_DEVICE_CHECKPOINT_TRANSACTION_HEADER_LEN {
            return Err(
                VersionedTwoDeviceCheckpointTransactionError::InvalidHeaderLength(header_len),
            );
        }
        let declared_total =
            u64::from_le_bytes(bytes[16..24].try_into().expect("fixed total-length field"));
        if usize::try_from(declared_total).ok() != Some(bytes.len()) {
            return Err(
                VersionedTwoDeviceCheckpointTransactionError::InvalidTotalLength {
                    declared: declared_total,
                    actual: bytes.len(),
                },
            );
        }
        let checkpoint_len =
            u64::from_le_bytes(bytes[24..32].try_into().expect("fixed checkpoint-length field"));
        let registration_len = u64::from_le_bytes(
            bytes[32..40]
                .try_into()
                .expect("fixed registration-pair-length field"),
        );
        let flags = u32::from_le_bytes(bytes[40..44].try_into().expect("fixed flags field"));
        if flags != 0 {
            return Err(VersionedTwoDeviceCheckpointTransactionError::NonZeroFlags(
                flags,
            ));
        }
        let reserved =
            u32::from_le_bytes(bytes[44..48].try_into().expect("fixed reserved field"));
        if reserved != 0 {
            return Err(
                VersionedTwoDeviceCheckpointTransactionError::NonZeroReserved(reserved),
            );
        }

        let checkpoint_len = usize::try_from(checkpoint_len).map_err(|_| {
            VersionedTwoDeviceCheckpointTransactionError::InvalidCheckpointLength(checkpoint_len)
        })?;
        if checkpoint_len == 0 {
            return Err(
                VersionedTwoDeviceCheckpointTransactionError::InvalidCheckpointLength(0),
            );
        }
        if registration_len
            != crate::kvm::sys::VERSIONED_HOST_REGISTRATION_PAIR_LEN as u64
        {
            return Err(
                VersionedTwoDeviceCheckpointTransactionError::InvalidRegistrationPairLength(
                    registration_len,
                ),
            );
        }
        let registration_len = usize::try_from(registration_len).map_err(|_| {
            VersionedTwoDeviceCheckpointTransactionError::InvalidRegistrationPairLength(
                registration_len,
            )
        })?;
        let expected_len = TWO_DEVICE_CHECKPOINT_TRANSACTION_HEADER_LEN
            .checked_add(checkpoint_len)
            .and_then(|length| length.checked_add(registration_len))
            .ok_or(VersionedTwoDeviceCheckpointTransactionError::LengthOverflow)?;
        if expected_len != bytes.len() {
            return Err(
                VersionedTwoDeviceCheckpointTransactionError::InvalidTotalLength {
                    declared: declared_total,
                    actual: expected_len,
                },
            );
        }

        let checkpoint_start = TWO_DEVICE_CHECKPOINT_TRANSACTION_HEADER_LEN;
        let checkpoint_end = checkpoint_start + checkpoint_len;
        Ok(Self {
            checkpoint: VersionedFullControllerTwoVirtioBlkCheckpointV1::decode(
                &bytes[checkpoint_start..checkpoint_end],
            )?,
            registrations: crate::kvm::sys::VersionedHostRegistrationPairV1::decode(
                &bytes[checkpoint_end..],
            )?,
        })
    }

    pub(crate) fn materialize(
        &self,
        host_msrs: &crate::kvm::msr::HostMsrIndexList,
    ) -> Result<
        (
            BoundedFullControllerTwoVirtioBlkCheckpoint,
            crate::kvm::sys::HostRegistrationSpecPair,
        ),
        VersionedTwoDeviceCheckpointTransactionError,
    > {
        Ok((
            self.checkpoint.materialize(host_msrs)?,
            self.registrations.materialize()?,
        ))
    }
}

#[cfg(test)]
mod versioned_two_device_checkpoint_transaction_tests {
    use super::*;

    fn minimally_sized_outer_envelope() -> Vec<u8> {
        let checkpoint_len = 1_usize;
        let registration_len = crate::kvm::sys::VERSIONED_HOST_REGISTRATION_PAIR_LEN;
        let total_len =
            TWO_DEVICE_CHECKPOINT_TRANSACTION_HEADER_LEN + checkpoint_len + registration_len;
        let mut bytes = vec![0_u8; total_len];
        bytes[0..8].copy_from_slice(&VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_MAGIC);
        bytes[8..10]
            .copy_from_slice(&VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_VERSION.to_le_bytes());
        bytes[10..12].copy_from_slice(
            &VERSIONED_TWO_DEVICE_CHECKPOINT_TRANSACTION_ARCH_X86_64.to_le_bytes(),
        );
        bytes[12..16].copy_from_slice(
            &(TWO_DEVICE_CHECKPOINT_TRANSACTION_HEADER_LEN as u32).to_le_bytes(),
        );
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
            VersionedTwoDeviceCheckpointTransactionV1::decode(&bad_magic),
            Err(VersionedTwoDeviceCheckpointTransactionError::InvalidMagic)
        );

        let mut bad_version = base.clone();
        bad_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedTwoDeviceCheckpointTransactionV1::decode(&bad_version),
            Err(
                VersionedTwoDeviceCheckpointTransactionError::UnsupportedVersion(2)
            )
        );

        let mut bad_arch = base.clone();
        bad_arch[10..12].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedTwoDeviceCheckpointTransactionV1::decode(&bad_arch),
            Err(
                VersionedTwoDeviceCheckpointTransactionError::UnsupportedArchitecture(2)
            )
        );

        let mut bad_header = base.clone();
        bad_header[12..16].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            VersionedTwoDeviceCheckpointTransactionV1::decode(&bad_header),
            Err(
                VersionedTwoDeviceCheckpointTransactionError::InvalidHeaderLength(0)
            )
        );

        let mut bad_total = base.clone();
        bad_total[16..24].copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            VersionedTwoDeviceCheckpointTransactionV1::decode(&bad_total),
            Err(
                VersionedTwoDeviceCheckpointTransactionError::InvalidTotalLength {
                    declared: 0,
                    actual: base.len(),
                }
            )
        );

        let mut bad_checkpoint_len = base.clone();
        bad_checkpoint_len[24..32].copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            VersionedTwoDeviceCheckpointTransactionV1::decode(&bad_checkpoint_len),
            Err(
                VersionedTwoDeviceCheckpointTransactionError::InvalidCheckpointLength(0)
            )
        );

        let mut bad_registration_len = base.clone();
        bad_registration_len[32..40].copy_from_slice(&1_u64.to_le_bytes());
        assert_eq!(
            VersionedTwoDeviceCheckpointTransactionV1::decode(&bad_registration_len),
            Err(
                VersionedTwoDeviceCheckpointTransactionError::InvalidRegistrationPairLength(1)
            )
        );

        let mut bad_flags = base.clone();
        bad_flags[40..44].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedTwoDeviceCheckpointTransactionV1::decode(&bad_flags),
            Err(VersionedTwoDeviceCheckpointTransactionError::NonZeroFlags(1))
        );

        let mut bad_reserved = base.clone();
        bad_reserved[44..48].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedTwoDeviceCheckpointTransactionV1::decode(&bad_reserved),
            Err(
                VersionedTwoDeviceCheckpointTransactionError::NonZeroReserved(1)
            )
        );

        let mut truncated = base;
        truncated.pop();
        assert!(matches!(
            VersionedTwoDeviceCheckpointTransactionV1::decode(&truncated),
            Err(VersionedTwoDeviceCheckpointTransactionError::InvalidTotalLength { .. })
        ));
    }
}
