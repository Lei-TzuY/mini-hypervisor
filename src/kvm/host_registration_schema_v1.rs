use std::fmt;

pub(crate) const VERSIONED_HOST_REGISTRATION_SPEC_VERSION: u16 = 1;
const VERSIONED_HOST_REGISTRATION_SPEC_MAGIC: [u8; 8] = *b"MHVHREG\0";
const VERSIONED_HOST_REGISTRATION_SPEC_LEN: usize = 48;
const VERSIONED_HOST_REGISTRATION_SPEC_FLAGS: u32 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VersionedHostRegistrationSpecError {
    Truncated { actual: usize },
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidHeaderLength(u16),
    InvalidTotalLength(u32),
    NonZeroFlags(u32),
    NonZeroReserved(u32),
    InvalidSpec(String),
}

impl fmt::Display for VersionedHostRegistrationSpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { actual } => write!(
                f,
                "versioned host-registration spec is truncated: {actual} bytes, expected {VERSIONED_HOST_REGISTRATION_SPEC_LEN}"
            ),
            Self::InvalidMagic => write!(f, "versioned host-registration spec has invalid magic"),
            Self::UnsupportedVersion(version) => write!(
                f,
                "versioned host-registration spec version {version} is unsupported"
            ),
            Self::InvalidHeaderLength(length) => write!(
                f,
                "versioned host-registration header length {length} is invalid"
            ),
            Self::InvalidTotalLength(length) => write!(
                f,
                "versioned host-registration total length {length} is invalid"
            ),
            Self::NonZeroFlags(flags) => write!(
                f,
                "versioned host-registration flags must be zero, got {flags:#x}"
            ),
            Self::NonZeroReserved(reserved) => write!(
                f,
                "versioned host-registration reserved field must be zero, got {reserved:#x}"
            ),
            Self::InvalidSpec(detail) => write!(
                f,
                "versioned host-registration semantic spec is invalid: {detail}"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VersionedHostRegistrationSpecV1 {
    spec: HostRegistrationSpec,
}

impl VersionedHostRegistrationSpecV1 {
    #[must_use]
    pub(crate) const fn from_spec(spec: HostRegistrationSpec) -> Self {
        Self { spec }
    }

    #[must_use]
    pub(crate) const fn version(&self) -> u16 {
        VERSIONED_HOST_REGISTRATION_SPEC_VERSION
    }

    #[must_use]
    pub(crate) const fn spec(&self) -> HostRegistrationSpec {
        self.spec
    }

    #[must_use]
    pub(crate) fn encode(&self) -> [u8; VERSIONED_HOST_REGISTRATION_SPEC_LEN] {
        let mut bytes = [0_u8; VERSIONED_HOST_REGISTRATION_SPEC_LEN];
        bytes[0..8].copy_from_slice(&VERSIONED_HOST_REGISTRATION_SPEC_MAGIC);
        bytes[8..10].copy_from_slice(&VERSIONED_HOST_REGISTRATION_SPEC_VERSION.to_le_bytes());
        bytes[10..12].copy_from_slice(&(VERSIONED_HOST_REGISTRATION_SPEC_LEN as u16).to_le_bytes());
        bytes[12..16].copy_from_slice(&(VERSIONED_HOST_REGISTRATION_SPEC_LEN as u32).to_le_bytes());
        bytes[16..24].copy_from_slice(&self.spec.doorbell_address().to_le_bytes());
        bytes[24..28].copy_from_slice(&self.spec.doorbell_length().to_le_bytes());
        bytes[28..32].copy_from_slice(&self.spec.gsi().to_le_bytes());
        bytes[32..40].copy_from_slice(&self.spec.doorbell_datamatch().to_le_bytes());
        bytes[40..44].copy_from_slice(&VERSIONED_HOST_REGISTRATION_SPEC_FLAGS.to_le_bytes());
        bytes[44..48].copy_from_slice(&0_u32.to_le_bytes());
        bytes
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, VersionedHostRegistrationSpecError> {
        if bytes.len() != VERSIONED_HOST_REGISTRATION_SPEC_LEN {
            return Err(VersionedHostRegistrationSpecError::Truncated {
                actual: bytes.len(),
            });
        }
        if bytes[0..8] != VERSIONED_HOST_REGISTRATION_SPEC_MAGIC {
            return Err(VersionedHostRegistrationSpecError::InvalidMagic);
        }

        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version slice"));
        if version != VERSIONED_HOST_REGISTRATION_SPEC_VERSION {
            return Err(VersionedHostRegistrationSpecError::UnsupportedVersion(version));
        }
        let header_len =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed header-length slice"));
        if usize::from(header_len) != VERSIONED_HOST_REGISTRATION_SPEC_LEN {
            return Err(VersionedHostRegistrationSpecError::InvalidHeaderLength(
                header_len,
            ));
        }
        let total_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed total-length slice"));
        if usize::try_from(total_len).ok() != Some(VERSIONED_HOST_REGISTRATION_SPEC_LEN) {
            return Err(VersionedHostRegistrationSpecError::InvalidTotalLength(
                total_len,
            ));
        }

        let doorbell_address =
            u64::from_le_bytes(bytes[16..24].try_into().expect("fixed doorbell-address slice"));
        let doorbell_length =
            u32::from_le_bytes(bytes[24..28].try_into().expect("fixed doorbell-length slice"));
        let gsi = u32::from_le_bytes(bytes[28..32].try_into().expect("fixed GSI slice"));
        let doorbell_datamatch =
            u64::from_le_bytes(bytes[32..40].try_into().expect("fixed datamatch slice"));
        let flags = u32::from_le_bytes(bytes[40..44].try_into().expect("fixed flags slice"));
        if flags != 0 {
            return Err(VersionedHostRegistrationSpecError::NonZeroFlags(flags));
        }
        let reserved =
            u32::from_le_bytes(bytes[44..48].try_into().expect("fixed reserved slice"));
        if reserved != 0 {
            return Err(VersionedHostRegistrationSpecError::NonZeroReserved(
                reserved,
            ));
        }

        let spec = HostRegistrationSpec::new(
            doorbell_address,
            doorbell_length,
            doorbell_datamatch,
            gsi,
        )
        .map_err(|error| VersionedHostRegistrationSpecError::InvalidSpec(error.to_string()))?;
        Ok(Self { spec })
    }
}

#[cfg(test)]
mod versioned_host_registration_spec_tests {
    use super::*;

    fn fixture_spec() -> HostRegistrationSpec {
        HostRegistrationSpec::new(0x1000_0100, 2, 0, 0).unwrap()
    }

    #[test]
    fn fd_free_registration_spec_round_trips_canonically() {
        let schema = VersionedHostRegistrationSpecV1::from_spec(fixture_spec());
        let bytes = schema.encode();
        assert_eq!(bytes.len(), VERSIONED_HOST_REGISTRATION_SPEC_LEN);
        let decoded = VersionedHostRegistrationSpecV1::decode(&bytes).unwrap();
        assert_eq!(decoded.spec(), fixture_spec());
        assert_eq!(decoded.encode(), bytes);
    }

    #[test]
    fn registration_envelope_and_semantics_fail_closed() {
        let base = VersionedHostRegistrationSpecV1::from_spec(fixture_spec()).encode();

        let mut bad_magic = base;
        bad_magic[0] ^= 0xff;
        assert_eq!(
            VersionedHostRegistrationSpecV1::decode(&bad_magic),
            Err(VersionedHostRegistrationSpecError::InvalidMagic)
        );

        let mut bad_version = base;
        bad_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationSpecV1::decode(&bad_version),
            Err(VersionedHostRegistrationSpecError::UnsupportedVersion(2))
        );

        let mut bad_header = base;
        bad_header[10..12].copy_from_slice(&0_u16.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationSpecV1::decode(&bad_header),
            Err(VersionedHostRegistrationSpecError::InvalidHeaderLength(0))
        );

        let mut bad_total = base;
        bad_total[12..16].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationSpecV1::decode(&bad_total),
            Err(VersionedHostRegistrationSpecError::InvalidTotalLength(0))
        );

        let mut bad_flags = base;
        bad_flags[40..44].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationSpecV1::decode(&bad_flags),
            Err(VersionedHostRegistrationSpecError::NonZeroFlags(1))
        );

        let mut bad_reserved = base;
        bad_reserved[44..48].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationSpecV1::decode(&bad_reserved),
            Err(VersionedHostRegistrationSpecError::NonZeroReserved(1))
        );

        let mut bad_length = base;
        bad_length[24..28].copy_from_slice(&3_u32.to_le_bytes());
        assert!(matches!(
            VersionedHostRegistrationSpecV1::decode(&bad_length),
            Err(VersionedHostRegistrationSpecError::InvalidSpec(_))
        ));

        assert_eq!(
            VersionedHostRegistrationSpecV1::decode(&base[..47]),
            Err(VersionedHostRegistrationSpecError::Truncated { actual: 47 })
        );
    }
}
