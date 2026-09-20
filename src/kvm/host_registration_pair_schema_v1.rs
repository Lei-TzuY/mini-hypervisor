pub const VERSIONED_HOST_REGISTRATION_PAIR_VERSION: u16 = 1;
const VERSIONED_HOST_REGISTRATION_PAIR_MAGIC: [u8; 8] = *b"MHVHR2\0\0";
const VERSIONED_HOST_REGISTRATION_PAIR_HEADER_LEN: usize = 32;
const VERSIONED_HOST_REGISTRATION_PAIR_COUNT: u16 = 2;
const VERSIONED_HOST_REGISTRATION_PAIR_FLAGS: u16 = 0;
const VERSIONED_HOST_REGISTRATION_PAIR_LEN: usize =
    VERSIONED_HOST_REGISTRATION_PAIR_HEADER_LEN + 2 * VERSIONED_HOST_REGISTRATION_SPEC_LEN;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VersionedHostRegistrationPairError {
    Truncated { actual: usize },
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidHeaderLength(u16),
    InvalidTotalLength(u32),
    InvalidRegistrationCount(u16),
    NonZeroFlags(u16),
    NonZeroReserved(u32),
    InvalidRegistrationLength { index: u8, length: u32 },
    Registration {
        index: u8,
        error: VersionedHostRegistrationSpecError,
    },
    NonCanonicalOrder,
    InvalidPair(String),
}

impl std::fmt::Display for VersionedHostRegistrationPairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { actual } => write!(
                f,
                "versioned host-registration pair is truncated: {actual} bytes, expected {VERSIONED_HOST_REGISTRATION_PAIR_LEN}"
            ),
            Self::InvalidMagic => {
                write!(f, "versioned host-registration pair has invalid magic")
            }
            Self::UnsupportedVersion(version) => write!(
                f,
                "versioned host-registration pair version {version} is unsupported"
            ),
            Self::InvalidHeaderLength(length) => write!(
                f,
                "versioned host-registration pair header length {length} is invalid"
            ),
            Self::InvalidTotalLength(length) => write!(
                f,
                "versioned host-registration pair total length {length} is invalid"
            ),
            Self::InvalidRegistrationCount(count) => write!(
                f,
                "versioned host-registration pair count {count} is not {VERSIONED_HOST_REGISTRATION_PAIR_COUNT}"
            ),
            Self::NonZeroFlags(flags) => write!(
                f,
                "versioned host-registration pair flags must be zero, got {flags:#x}"
            ),
            Self::NonZeroReserved(reserved) => write!(
                f,
                "versioned host-registration pair reserved field must be zero, got {reserved:#x}"
            ),
            Self::InvalidRegistrationLength { index, length } => write!(
                f,
                "versioned host-registration pair member {index} length {length} is not {VERSIONED_HOST_REGISTRATION_SPEC_LEN}"
            ),
            Self::Registration { index, error } => {
                write!(f, "versioned host-registration pair member {index} is invalid: {error}")
            }
            Self::NonCanonicalOrder => write!(
                f,
                "versioned host-registration pair members are not in ascending doorbell order"
            ),
            Self::InvalidPair(detail) => {
                write!(f, "versioned host-registration pair semantics are invalid: {detail}")
            }
        }
    }
}

impl std::error::Error for VersionedHostRegistrationPairError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Registration { error, .. } => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VersionedHostRegistrationPairV1 {
    pair: HostRegistrationSpecPair,
}

impl VersionedHostRegistrationPairV1 {
    #[must_use]
    pub(crate) const fn from_pair(pair: HostRegistrationSpecPair) -> Self {
        Self { pair }
    }

    #[must_use]
    pub(crate) const fn version(&self) -> u16 {
        VERSIONED_HOST_REGISTRATION_PAIR_VERSION
    }

    #[must_use]
    pub(crate) const fn registration_versions(&self) -> [u16; 2] {
        [
            VERSIONED_HOST_REGISTRATION_SPEC_VERSION,
            VERSIONED_HOST_REGISTRATION_SPEC_VERSION,
        ]
    }

    #[must_use]
    pub(crate) fn encode(&self) -> [u8; VERSIONED_HOST_REGISTRATION_PAIR_LEN] {
        let specs = self.pair.specs();
        let first = VersionedHostRegistrationSpecV1::from_spec(specs[0]).encode();
        let second = VersionedHostRegistrationSpecV1::from_spec(specs[1]).encode();
        let mut bytes = [0_u8; VERSIONED_HOST_REGISTRATION_PAIR_LEN];
        bytes[0..8].copy_from_slice(&VERSIONED_HOST_REGISTRATION_PAIR_MAGIC);
        bytes[8..10].copy_from_slice(&VERSIONED_HOST_REGISTRATION_PAIR_VERSION.to_le_bytes());
        bytes[10..12]
            .copy_from_slice(&(VERSIONED_HOST_REGISTRATION_PAIR_HEADER_LEN as u16).to_le_bytes());
        bytes[12..16]
            .copy_from_slice(&(VERSIONED_HOST_REGISTRATION_PAIR_LEN as u32).to_le_bytes());
        bytes[16..18].copy_from_slice(&VERSIONED_HOST_REGISTRATION_PAIR_COUNT.to_le_bytes());
        bytes[18..20].copy_from_slice(&VERSIONED_HOST_REGISTRATION_PAIR_FLAGS.to_le_bytes());
        bytes[20..24].copy_from_slice(&0_u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&(VERSIONED_HOST_REGISTRATION_SPEC_LEN as u32).to_le_bytes());
        bytes[28..32].copy_from_slice(&(VERSIONED_HOST_REGISTRATION_SPEC_LEN as u32).to_le_bytes());
        bytes[32..32 + VERSIONED_HOST_REGISTRATION_SPEC_LEN].copy_from_slice(&first);
        bytes[32 + VERSIONED_HOST_REGISTRATION_SPEC_LEN..].copy_from_slice(&second);
        bytes
    }

    pub(crate) fn decode(
        bytes: &[u8],
    ) -> Result<Self, VersionedHostRegistrationPairError> {
        if bytes.len() != VERSIONED_HOST_REGISTRATION_PAIR_LEN {
            return Err(VersionedHostRegistrationPairError::Truncated {
                actual: bytes.len(),
            });
        }
        if bytes[0..8] != VERSIONED_HOST_REGISTRATION_PAIR_MAGIC {
            return Err(VersionedHostRegistrationPairError::InvalidMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version slice"));
        if version != VERSIONED_HOST_REGISTRATION_PAIR_VERSION {
            return Err(VersionedHostRegistrationPairError::UnsupportedVersion(
                version,
            ));
        }
        let header_len =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed header-length slice"));
        if usize::from(header_len) != VERSIONED_HOST_REGISTRATION_PAIR_HEADER_LEN {
            return Err(VersionedHostRegistrationPairError::InvalidHeaderLength(
                header_len,
            ));
        }
        let total_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed total-length slice"));
        if usize::try_from(total_len).ok() != Some(VERSIONED_HOST_REGISTRATION_PAIR_LEN) {
            return Err(VersionedHostRegistrationPairError::InvalidTotalLength(
                total_len,
            ));
        }
        let count = u16::from_le_bytes(bytes[16..18].try_into().expect("fixed count slice"));
        if count != VERSIONED_HOST_REGISTRATION_PAIR_COUNT {
            return Err(VersionedHostRegistrationPairError::InvalidRegistrationCount(
                count,
            ));
        }
        let flags = u16::from_le_bytes(bytes[18..20].try_into().expect("fixed flags slice"));
        if flags != 0 {
            return Err(VersionedHostRegistrationPairError::NonZeroFlags(flags));
        }
        let reserved =
            u32::from_le_bytes(bytes[20..24].try_into().expect("fixed reserved slice"));
        if reserved != 0 {
            return Err(VersionedHostRegistrationPairError::NonZeroReserved(
                reserved,
            ));
        }
        for (index, range) in [(0_u8, 24..28), (1_u8, 28..32)] {
            let length =
                u32::from_le_bytes(bytes[range].try_into().expect("fixed member-length slice"));
            if usize::try_from(length).ok() != Some(VERSIONED_HOST_REGISTRATION_SPEC_LEN) {
                return Err(
                    VersionedHostRegistrationPairError::InvalidRegistrationLength {
                        index,
                        length,
                    },
                );
            }
        }

        let first_end = VERSIONED_HOST_REGISTRATION_PAIR_HEADER_LEN
            + VERSIONED_HOST_REGISTRATION_SPEC_LEN;
        let first_schema = VersionedHostRegistrationSpecV1::decode(
            &bytes[VERSIONED_HOST_REGISTRATION_PAIR_HEADER_LEN..first_end],
        )
        .map_err(|error| VersionedHostRegistrationPairError::Registration {
            index: 0,
            error,
        })?;
        let second_schema = VersionedHostRegistrationSpecV1::decode(&bytes[first_end..])
            .map_err(|error| VersionedHostRegistrationPairError::Registration {
                index: 1,
                error,
            })?;
        let first = first_schema.spec();
        let second = second_schema.spec();
        if first.doorbell_address() >= second.doorbell_address() {
            return Err(VersionedHostRegistrationPairError::NonCanonicalOrder);
        }
        let pair = HostRegistrationSpecPair::new([first, second]).map_err(|error| {
            VersionedHostRegistrationPairError::InvalidPair(error.to_string())
        })?;
        if pair.specs() != [first, second] {
            return Err(VersionedHostRegistrationPairError::NonCanonicalOrder);
        }
        Ok(Self { pair })
    }

    pub(crate) fn materialize(
        &self,
    ) -> Result<HostRegistrationSpecPair, VersionedHostRegistrationPairError> {
        HostRegistrationSpecPair::new(self.pair.specs()).map_err(|error| {
            VersionedHostRegistrationPairError::InvalidPair(error.to_string())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedTwoHostRegistrationAccelerationResult {
    acceleration: TwoHostRegistrationAccelerationResult,
    schema_version: u16,
    encoded_len: usize,
    registration_versions: [u16; 2],
    canonical_roundtrip: bool,
}

impl VersionedTwoHostRegistrationAccelerationResult {
    #[must_use]
    pub const fn acceleration(&self) -> &TwoHostRegistrationAccelerationResult {
        &self.acceleration
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn encoded_len(&self) -> usize {
        self.encoded_len
    }

    #[must_use]
    pub const fn registration_versions(&self) -> [u16; 2] {
        self.registration_versions
    }

    #[must_use]
    pub const fn canonical_roundtrip(&self) -> bool {
        self.canonical_roundtrip
    }
}

impl KvmBackend {
    pub fn run_versioned_two_host_registration_acceleration_guest(
        config: crate::config::VmConfig,
    ) -> Result<VersionedTwoHostRegistrationAccelerationResult, crate::error::Error> {
        let encoded = {
            let pair = default_two_host_registration_pair()?;
            VersionedHostRegistrationPairV1::from_pair(pair).encode()
        };
        let encoded_len = encoded.len();

        // The encoder-side semantic pair is out of scope here. Everything below this boundary is
        // reconstructed only from the canonical byte stream.
        let decoded = VersionedHostRegistrationPairV1::decode(&encoded).map_err(|error| {
            host_registration_error(
                "decode versioned host-registration pair",
                error.to_string(),
            )
        })?;
        let canonical = decoded.encode();
        if canonical != encoded {
            return Err(host_registration_error(
                "re-encode versioned host-registration pair",
                "decoded pair did not reproduce the canonical byte stream",
            ));
        }
        let schema_version = decoded.version();
        let registration_versions = decoded.registration_versions();
        let materialized = decoded.materialize().map_err(|error| {
            host_registration_error(
                "materialize versioned host-registration pair",
                error.to_string(),
            )
        })?;
        let acceleration =
            run_two_host_registration_acceleration_guest_with_pair(config, materialized)?;

        Ok(VersionedTwoHostRegistrationAccelerationResult {
            acceleration,
            schema_version,
            encoded_len,
            registration_versions,
            canonical_roundtrip: true,
        })
    }
}

#[cfg(test)]
mod versioned_host_registration_pair_tests {
    use super::*;

    fn fixture_pair() -> HostRegistrationSpecPair {
        HostRegistrationSpecPair::new([
            HostRegistrationSpec::new(0x1000_1100, 2, 0, 1).unwrap(),
            HostRegistrationSpec::new(0x1000_0100, 2, 0, 0).unwrap(),
        ])
        .unwrap()
    }

    #[test]
    fn pair_schema_round_trips_canonically() {
        let schema = VersionedHostRegistrationPairV1::from_pair(fixture_pair());
        let encoded = schema.encode();
        assert_eq!(encoded.len(), VERSIONED_HOST_REGISTRATION_PAIR_LEN);
        let decoded = VersionedHostRegistrationPairV1::decode(&encoded).unwrap();
        assert_eq!(decoded.encode(), encoded);
        assert_eq!(decoded.registration_versions(), [1, 1]);
        let materialized = decoded.materialize().unwrap();
        let specs = materialized.specs();
        assert_eq!(
            [specs[0].doorbell_address(), specs[1].doorbell_address()],
            [0x1000_0100, 0x1000_1100]
        );
        assert_eq!([specs[0].gsi(), specs[1].gsi()], [0, 1]);
        assert_eq!(materialized, fixture_pair());
    }

    #[test]
    fn pair_envelope_fails_closed_on_header_and_nested_corruption() {
        let base = VersionedHostRegistrationPairV1::from_pair(fixture_pair()).encode();

        let mut bad_magic = base;
        bad_magic[0] ^= 0xff;
        assert_eq!(
            VersionedHostRegistrationPairV1::decode(&bad_magic),
            Err(VersionedHostRegistrationPairError::InvalidMagic)
        );

        let mut bad_version = base;
        bad_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationPairV1::decode(&bad_version),
            Err(VersionedHostRegistrationPairError::UnsupportedVersion(2))
        );

        let mut bad_header = base;
        bad_header[10..12].copy_from_slice(&0_u16.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationPairV1::decode(&bad_header),
            Err(VersionedHostRegistrationPairError::InvalidHeaderLength(0))
        );

        let mut bad_total = base;
        bad_total[12..16].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationPairV1::decode(&bad_total),
            Err(VersionedHostRegistrationPairError::InvalidTotalLength(0))
        );

        let mut bad_count = base;
        bad_count[16..18].copy_from_slice(&1_u16.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationPairV1::decode(&bad_count),
            Err(VersionedHostRegistrationPairError::InvalidRegistrationCount(1))
        );

        let mut bad_flags = base;
        bad_flags[18..20].copy_from_slice(&1_u16.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationPairV1::decode(&bad_flags),
            Err(VersionedHostRegistrationPairError::NonZeroFlags(1))
        );

        let mut bad_reserved = base;
        bad_reserved[20..24].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationPairV1::decode(&bad_reserved),
            Err(VersionedHostRegistrationPairError::NonZeroReserved(1))
        );

        let mut bad_first_len = base;
        bad_first_len[24..28].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            VersionedHostRegistrationPairV1::decode(&bad_first_len),
            Err(
                VersionedHostRegistrationPairError::InvalidRegistrationLength {
                    index: 0,
                    length: 0,
                }
            )
        );

        let mut bad_nested_version = base;
        bad_nested_version[40..42].copy_from_slice(&2_u16.to_le_bytes());
        assert!(matches!(
            VersionedHostRegistrationPairV1::decode(&bad_nested_version),
            Err(VersionedHostRegistrationPairError::Registration {
                index: 0,
                error: VersionedHostRegistrationSpecError::UnsupportedVersion(2),
            })
        ));

        assert_eq!(
            VersionedHostRegistrationPairV1::decode(&base[..base.len() - 1]),
            Err(VersionedHostRegistrationPairError::Truncated {
                actual: base.len() - 1,
            })
        );
    }

    #[test]
    fn pair_decode_rejects_noncanonical_overlap_and_duplicate_gsi() {
        let base = VersionedHostRegistrationPairV1::from_pair(fixture_pair()).encode();
        let first_range = 32..32 + VERSIONED_HOST_REGISTRATION_SPEC_LEN;
        let second_range = 32 + VERSIONED_HOST_REGISTRATION_SPEC_LEN
            ..32 + 2 * VERSIONED_HOST_REGISTRATION_SPEC_LEN;
        let first_bytes = base[first_range.clone()].to_vec();
        let second_bytes = base[second_range.clone()].to_vec();

        let mut swapped = base;
        swapped[first_range.clone()].copy_from_slice(&second_bytes);
        swapped[second_range.clone()].copy_from_slice(&first_bytes);
        assert_eq!(
            VersionedHostRegistrationPairV1::decode(&swapped),
            Err(VersionedHostRegistrationPairError::NonCanonicalOrder)
        );

        let mut overlap = base;
        overlap[96..104].copy_from_slice(&0x1000_0101_u64.to_le_bytes());
        assert!(matches!(
            VersionedHostRegistrationPairV1::decode(&overlap),
            Err(VersionedHostRegistrationPairError::InvalidPair(_))
        ));

        let mut duplicate_gsi = base;
        duplicate_gsi[108..112].copy_from_slice(&0_u32.to_le_bytes());
        assert!(matches!(
            VersionedHostRegistrationPairV1::decode(&duplicate_gsi),
            Err(VersionedHostRegistrationPairError::InvalidPair(_))
        ));
    }
}
