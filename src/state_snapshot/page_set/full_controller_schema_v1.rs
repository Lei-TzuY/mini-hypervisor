pub const VERSIONED_FULL_CONTROLLER_CHECKPOINT_MAGIC: [u8; 8] = *b"MHVFCTL\0";
pub const VERSIONED_FULL_CONTROLLER_CHECKPOINT_VERSION: u16 = 1;
pub const VERSIONED_FULL_CONTROLLER_CHECKPOINT_ARCH_X86_64: u16 = 1;

const FULL_CONTROLLER_SCHEMA_HEADER_LEN: usize = 40;
const FULL_CONTROLLER_SCHEMA_STATE_LEN: usize =
    2 * crate::kvm::sys::KVM_PIC_STATE_SIZE
        + crate::kvm::sys::KVM_IOAPIC_STATE_SIZE
        + crate::kvm::sys::KVM_APIC_REG_SIZE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionedFullControllerCheckpointError {
    InvalidMagic,
    UnsupportedVersion(u16),
    UnsupportedArchitecture(u16),
    InvalidHeaderLength(u32),
    InvalidTotalLength { declared: u64, actual: usize },
    InvalidGuestLength(u64),
    NonZeroFlags(u32),
    NonZeroReserved(u32),
    LengthOverflow,
    NonZeroIoapicPad(u32),
    Guest(VersionedPageVcpuCheckpointError),
}

impl std::fmt::Display for VersionedFullControllerCheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "full-controller checkpoint schema magic does not match MHVFCTL"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported full-controller checkpoint schema version {version}")
            }
            Self::UnsupportedArchitecture(architecture) => write!(
                f,
                "unsupported full-controller checkpoint schema architecture identifier {architecture}"
            ),
            Self::InvalidHeaderLength(length) => write!(
                f,
                "full-controller checkpoint header length {length} is not {FULL_CONTROLLER_SCHEMA_HEADER_LEN}"
            ),
            Self::InvalidTotalLength { declared, actual } => write!(
                f,
                "full-controller checkpoint declares total length {declared}, actual byte length is {actual}"
            ),
            Self::InvalidGuestLength(length) => write!(
                f,
                "full-controller checkpoint nested page+vCPU payload length {length} is invalid"
            ),
            Self::NonZeroFlags(flags) => write!(
                f,
                "full-controller checkpoint v1 flags must be zero, got {flags:#x}"
            ),
            Self::NonZeroReserved(value) => write!(
                f,
                "full-controller checkpoint reserved field must be zero, got {value:#x}"
            ),
            Self::LengthOverflow => write!(f, "full-controller checkpoint length arithmetic overflowed"),
            Self::NonZeroIoapicPad(value) => write!(
                f,
                "full-controller checkpoint IOAPIC pad must be zero, got {value:#x}"
            ),
            Self::Guest(error) => write!(f, "nested page+vCPU checkpoint is invalid: {error}"),
        }
    }
}

impl std::error::Error for VersionedFullControllerCheckpointError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Guest(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VersionedPageVcpuCheckpointError> for VersionedFullControllerCheckpointError {
    fn from(error: VersionedPageVcpuCheckpointError) -> Self {
        Self::Guest(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedFullControllerCheckpointV1 {
    guest: VersionedPageVcpuCheckpointV1,
    master_pic: crate::kvm::sys::MasterPicStateSnapshot,
    slave_pic: crate::kvm::sys::SlavePicStateSnapshot,
    ioapic: crate::kvm::sys::IoapicStateSnapshot,
    lapic: crate::kvm::sys::KvmLapicState,
}

impl VersionedFullControllerCheckpointV1 {
    pub fn from_checkpoint(
        checkpoint: &BoundedFullControllerCheckpoint,
    ) -> Result<Self, VersionedFullControllerCheckpointError> {
        let guest = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint.base.guest)?;
        if checkpoint.ioapic.pad() != 0 {
            return Err(VersionedFullControllerCheckpointError::NonZeroIoapicPad(
                checkpoint.ioapic.pad(),
            ));
        }
        Ok(Self {
            guest,
            master_pic: checkpoint.master_pic().clone(),
            slave_pic: checkpoint.slave_pic().clone(),
            ioapic: checkpoint.ioapic().clone(),
            lapic: checkpoint.lapic().clone(),
        })
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        VERSIONED_FULL_CONTROLLER_CHECKPOINT_VERSION
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.guest.pages().len()
    }

    #[must_use]
    pub fn msr_count(&self) -> usize {
        self.guest.msr_count()
    }

    pub fn encode(&self) -> Result<Vec<u8>, VersionedFullControllerCheckpointError> {
        if self.ioapic.pad() != 0 {
            return Err(VersionedFullControllerCheckpointError::NonZeroIoapicPad(
                self.ioapic.pad(),
            ));
        }
        let guest = self.guest.encode()?;
        let total_len = FULL_CONTROLLER_SCHEMA_HEADER_LEN
            .checked_add(guest.len())
            .and_then(|length| length.checked_add(FULL_CONTROLLER_SCHEMA_STATE_LEN))
            .ok_or(VersionedFullControllerCheckpointError::LengthOverflow)?;

        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_FULL_CONTROLLER_CHECKPOINT_MAGIC);
        bytes.extend_from_slice(&VERSIONED_FULL_CONTROLLER_CHECKPOINT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&VERSIONED_FULL_CONTROLLER_CHECKPOINT_ARCH_X86_64.to_le_bytes());
        bytes.extend_from_slice(&(FULL_CONTROLLER_SCHEMA_HEADER_LEN as u32).to_le_bytes());
        bytes.extend_from_slice(&(total_len as u64).to_le_bytes());
        bytes.extend_from_slice(&(guest.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());

        bytes.extend_from_slice(&guest);
        bytes.extend_from_slice(&self.master_pic.semantic_bytes());
        bytes.extend_from_slice(&self.slave_pic.semantic_bytes());
        bytes.extend_from_slice(&self.ioapic.semantic_bytes());
        bytes.extend_from_slice(&self.lapic.regs);
        debug_assert_eq!(bytes.len(), total_len);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, VersionedFullControllerCheckpointError> {
        if bytes.len() < FULL_CONTROLLER_SCHEMA_HEADER_LEN {
            return Err(VersionedFullControllerCheckpointError::InvalidTotalLength {
                declared: 0,
                actual: bytes.len(),
            });
        }
        if bytes[0..8] != VERSIONED_FULL_CONTROLLER_CHECKPOINT_MAGIC {
            return Err(VersionedFullControllerCheckpointError::InvalidMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version field"));
        if version != VERSIONED_FULL_CONTROLLER_CHECKPOINT_VERSION {
            return Err(VersionedFullControllerCheckpointError::UnsupportedVersion(version));
        }
        let architecture =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed architecture field"));
        if architecture != VERSIONED_FULL_CONTROLLER_CHECKPOINT_ARCH_X86_64 {
            return Err(VersionedFullControllerCheckpointError::UnsupportedArchitecture(
                architecture,
            ));
        }
        let header_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header length field"));
        if header_len != FULL_CONTROLLER_SCHEMA_HEADER_LEN as u32 {
            return Err(VersionedFullControllerCheckpointError::InvalidHeaderLength(
                header_len,
            ));
        }
        let declared_total =
            u64::from_le_bytes(bytes[16..24].try_into().expect("fixed total length field"));
        if declared_total != bytes.len() as u64 {
            return Err(VersionedFullControllerCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: bytes.len(),
            });
        }
        let guest_len =
            u64::from_le_bytes(bytes[24..32].try_into().expect("fixed guest length field"));
        let flags = u32::from_le_bytes(bytes[32..36].try_into().expect("fixed flags field"));
        if flags != 0 {
            return Err(VersionedFullControllerCheckpointError::NonZeroFlags(flags));
        }
        let reserved =
            u32::from_le_bytes(bytes[36..40].try_into().expect("fixed reserved field"));
        if reserved != 0 {
            return Err(VersionedFullControllerCheckpointError::NonZeroReserved(
                reserved,
            ));
        }

        let guest_len = usize::try_from(guest_len)
            .map_err(|_| VersionedFullControllerCheckpointError::InvalidGuestLength(guest_len))?;
        if guest_len == 0 {
            return Err(VersionedFullControllerCheckpointError::InvalidGuestLength(0));
        }
        let expected_len = FULL_CONTROLLER_SCHEMA_HEADER_LEN
            .checked_add(guest_len)
            .and_then(|length| length.checked_add(FULL_CONTROLLER_SCHEMA_STATE_LEN))
            .ok_or(VersionedFullControllerCheckpointError::LengthOverflow)?;
        if expected_len != bytes.len() {
            return Err(VersionedFullControllerCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: expected_len,
            });
        }

        let mut offset = FULL_CONTROLLER_SCHEMA_HEADER_LEN;
        let guest_end = offset + guest_len;
        let guest = VersionedPageVcpuCheckpointV1::decode(&bytes[offset..guest_end])?;
        offset = guest_end;

        let master_end = offset + crate::kvm::sys::KVM_PIC_STATE_SIZE;
        let master_pic = crate::kvm::sys::MasterPicStateSnapshot::from_semantic_bytes(
            bytes[offset..master_end]
                .try_into()
                .expect("validated master PIC semantic length"),
        );
        offset = master_end;

        let slave_end = offset + crate::kvm::sys::KVM_PIC_STATE_SIZE;
        let slave_pic = crate::kvm::sys::SlavePicStateSnapshot::from_semantic_bytes(
            bytes[offset..slave_end]
                .try_into()
                .expect("validated slave PIC semantic length"),
        );
        offset = slave_end;

        let ioapic_end = offset + crate::kvm::sys::KVM_IOAPIC_STATE_SIZE;
        let ioapic = crate::kvm::sys::IoapicStateSnapshot::from_semantic_bytes(
            bytes[offset..ioapic_end]
                .try_into()
                .expect("validated IOAPIC semantic length"),
        );
        if ioapic.pad() != 0 {
            return Err(VersionedFullControllerCheckpointError::NonZeroIoapicPad(
                ioapic.pad(),
            ));
        }
        offset = ioapic_end;

        let lapic_end = offset + crate::kvm::sys::KVM_APIC_REG_SIZE;
        let lapic = crate::kvm::sys::KvmLapicState {
            regs: bytes[offset..lapic_end]
                .try_into()
                .expect("validated LAPIC state length"),
        };
        debug_assert_eq!(lapic_end, bytes.len());

        Ok(Self {
            guest,
            master_pic,
            slave_pic,
            ioapic,
            lapic,
        })
    }

    pub fn materialize(
        &self,
        host_msrs: &crate::kvm::msr::HostMsrIndexList,
    ) -> Result<BoundedFullControllerCheckpoint, VersionedFullControllerCheckpointError> {
        let guest = self.guest.materialize(host_msrs)?;
        if self.ioapic.pad() != 0 {
            return Err(VersionedFullControllerCheckpointError::NonZeroIoapicPad(
                self.ioapic.pad(),
            ));
        }
        Ok(BoundedFullControllerCheckpoint {
            base: BoundedControllerCheckpoint {
                guest,
                master_pic: self.master_pic.clone(),
                lapic: self.lapic.clone(),
            },
            slave_pic: self.slave_pic.clone(),
            ioapic: self.ioapic.clone(),
        })
    }
}

#[cfg(test)]
mod versioned_full_controller_schema_tests {
    use super::*;

    #[test]
    fn constants_define_fixed_controller_payload_without_uapi_padding() {
        assert_eq!(VERSIONED_FULL_CONTROLLER_CHECKPOINT_MAGIC, *b"MHVFCTL\0");
        assert_eq!(VERSIONED_FULL_CONTROLLER_CHECKPOINT_VERSION, 1);
        assert_eq!(VERSIONED_FULL_CONTROLLER_CHECKPOINT_ARCH_X86_64, 1);
        assert_eq!(FULL_CONTROLLER_SCHEMA_HEADER_LEN, 40);
        assert_eq!(
            FULL_CONTROLLER_SCHEMA_STATE_LEN,
            16 + 16 + 216 + crate::kvm::sys::KVM_APIC_REG_SIZE
        );
    }

    #[test]
    fn short_or_wrong_magic_payloads_fail_closed() {
        assert!(matches!(
            VersionedFullControllerCheckpointV1::decode(&[]),
            Err(VersionedFullControllerCheckpointError::InvalidTotalLength { .. })
        ));
        let mut bytes = vec![0_u8; FULL_CONTROLLER_SCHEMA_HEADER_LEN];
        bytes[0..8].copy_from_slice(b"BADMAGC\0");
        assert!(matches!(
            VersionedFullControllerCheckpointV1::decode(&bytes),
            Err(VersionedFullControllerCheckpointError::InvalidMagic)
        ));
    }
}
