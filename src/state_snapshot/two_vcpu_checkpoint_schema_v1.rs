use super::{BoundedTwoVcpuCheckpoint, BoundedTwoVcpuFullControllerCheckpoint};
use crate::kvm::msr::HostMsrIndexList;
use crate::kvm::sys::{
    IoapicStateSnapshot, KvmLapicState, MasterPicStateSnapshot, SlavePicStateSnapshot,
    KVM_APIC_REG_SIZE, KVM_IOAPIC_STATE_SIZE, KVM_PIC_STATE_SIZE,
};
use crate::state_snapshot::{
    VersionedPageVcpuCheckpointError, VersionedPageVcpuCheckpointV1,
    VersionedVcpuStateSnapshotError, VersionedVcpuStateSnapshotV1,
};
use crate::vcpu::VcpuId;

pub const VERSIONED_TWO_VCPU_FULL_CONTROLLER_MAGIC: [u8; 8] = *b"MHV2CTL\0";
pub const VERSIONED_TWO_VCPU_FULL_CONTROLLER_VERSION: u16 = 1;
pub const VERSIONED_TWO_VCPU_FULL_CONTROLLER_ARCH_X86_64: u16 = 1;

const TWO_VCPU_FULL_CONTROLLER_HEADER_LEN: usize = 64;
const TWO_VCPU_FULL_CONTROLLER_STATE_LEN: usize =
    2 * KVM_PIC_STATE_SIZE + KVM_IOAPIC_STATE_SIZE + 2 * KVM_APIC_REG_SIZE;
const X86_MP_STATE_MAX: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionedTwoVcpuFullControllerCheckpointError {
    InvalidMagic,
    UnsupportedVersion(u16),
    UnsupportedArchitecture(u16),
    InvalidHeaderLength(u32),
    InvalidTotalLength { declared: u64, actual: usize },
    InvalidPrimaryLength(u64),
    InvalidSecondaryLength(u64),
    NonCanonicalVcpuIds { primary: u16, secondary: u16 },
    InvalidMpState { vcpu: u16, state: u32 },
    NonZeroFlags(u32),
    NonZeroReserved(u64),
    LengthOverflow,
    NonZeroIoapicPad(u32),
    Primary(VersionedPageVcpuCheckpointError),
    Secondary(VersionedVcpuStateSnapshotError),
}

impl std::fmt::Display for VersionedTwoVcpuFullControllerCheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic => {
                write!(f, "two-vCPU controller schema magic does not match MHV2CTL")
            }
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported two-vCPU controller schema version {version}")
            }
            Self::UnsupportedArchitecture(architecture) => write!(
                f,
                "unsupported two-vCPU controller architecture identifier {architecture}"
            ),
            Self::InvalidHeaderLength(length) => write!(
                f,
                "two-vCPU controller header length {length} is not {TWO_VCPU_FULL_CONTROLLER_HEADER_LEN}"
            ),
            Self::InvalidTotalLength { declared, actual } => write!(
                f,
                "two-vCPU controller declares total length {declared}, actual byte length is {actual}"
            ),
            Self::InvalidPrimaryLength(length) => {
                write!(f, "two-vCPU controller primary checkpoint length {length} is invalid")
            }
            Self::InvalidSecondaryLength(length) => {
                write!(f, "two-vCPU controller secondary vCPU length {length} is invalid")
            }
            Self::NonCanonicalVcpuIds { primary, secondary } => write!(
                f,
                "two-vCPU controller ids must be strictly increasing, got [{primary}, {secondary}]"
            ),
            Self::InvalidMpState { vcpu, state } => write!(
                f,
                "two-vCPU controller vCPU {vcpu} has invalid x86 MP state {state}"
            ),
            Self::NonZeroFlags(flags) => {
                write!(f, "two-vCPU controller v1 flags must be zero, got {flags:#x}")
            }
            Self::NonZeroReserved(value) => {
                write!(f, "two-vCPU controller reserved field must be zero, got {value:#x}")
            }
            Self::LengthOverflow => write!(f, "two-vCPU controller length arithmetic overflowed"),
            Self::NonZeroIoapicPad(value) => {
                write!(f, "two-vCPU controller IOAPIC pad must be zero, got {value:#x}")
            }
            Self::Primary(error) => write!(f, "primary page+vCPU checkpoint is invalid: {error}"),
            Self::Secondary(error) => write!(f, "secondary vCPU checkpoint is invalid: {error}"),
        }
    }
}

impl std::error::Error for VersionedTwoVcpuFullControllerCheckpointError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Primary(error) => Some(error),
            Self::Secondary(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VersionedPageVcpuCheckpointError> for VersionedTwoVcpuFullControllerCheckpointError {
    fn from(error: VersionedPageVcpuCheckpointError) -> Self {
        Self::Primary(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedTwoVcpuFullControllerCheckpointV1 {
    primary: VersionedPageVcpuCheckpointV1,
    secondary: VersionedVcpuStateSnapshotV1,
    vcpu_ids: [VcpuId; 2],
    mp_states: [u32; 2],
    master_pic: MasterPicStateSnapshot,
    slave_pic: SlavePicStateSnapshot,
    ioapic: IoapicStateSnapshot,
    lapics: [KvmLapicState; 2],
}

impl VersionedTwoVcpuFullControllerCheckpointV1 {
    pub fn from_checkpoint(
        checkpoint: &BoundedTwoVcpuFullControllerCheckpoint,
    ) -> Result<Self, VersionedTwoVcpuFullControllerCheckpointError> {
        let ids = checkpoint.base.vcpu_ids();
        validate_vcpu_ids(ids)?;
        let mp_states = [checkpoint.mp_states[0].1, checkpoint.mp_states[1].1];
        validate_mp_states(ids, mp_states)?;
        if checkpoint.lapics[0].0 != ids[0]
            || checkpoint.lapics[1].0 != ids[1]
            || checkpoint.mp_states[0].0 != ids[0]
            || checkpoint.mp_states[1].0 != ids[1]
        {
            return Err(
                VersionedTwoVcpuFullControllerCheckpointError::NonCanonicalVcpuIds {
                    primary: ids[0].get(),
                    secondary: ids[1].get(),
                },
            );
        }
        if checkpoint.ioapic.pad() != 0 {
            return Err(
                VersionedTwoVcpuFullControllerCheckpointError::NonZeroIoapicPad(
                    checkpoint.ioapic.pad(),
                ),
            );
        }
        Ok(Self {
            primary: VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint.base.primary)?,
            secondary: VersionedVcpuStateSnapshotV1::from_snapshot(&checkpoint.base.secondary)
                .map_err(VersionedTwoVcpuFullControllerCheckpointError::Secondary)?,
            vcpu_ids: ids,
            mp_states,
            master_pic: checkpoint.master_pic.clone(),
            slave_pic: checkpoint.slave_pic.clone(),
            ioapic: checkpoint.ioapic.clone(),
            lapics: [
                checkpoint.lapics[0].1.clone(),
                checkpoint.lapics[1].1.clone(),
            ],
        })
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        VERSIONED_TWO_VCPU_FULL_CONTROLLER_VERSION
    }

    #[must_use]
    pub const fn vcpu_ids(&self) -> [VcpuId; 2] {
        self.vcpu_ids
    }

    #[must_use]
    pub const fn mp_states(&self) -> [u32; 2] {
        self.mp_states
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.primary.pages().len()
    }

    #[must_use]
    pub fn msr_counts(&self) -> [usize; 2] {
        [self.primary.msr_count(), self.secondary.msr_count()]
    }

    pub fn encode(&self) -> Result<Vec<u8>, VersionedTwoVcpuFullControllerCheckpointError> {
        validate_vcpu_ids(self.vcpu_ids)?;
        validate_mp_states(self.vcpu_ids, self.mp_states)?;
        if self.ioapic.pad() != 0 {
            return Err(
                VersionedTwoVcpuFullControllerCheckpointError::NonZeroIoapicPad(self.ioapic.pad()),
            );
        }
        let primary = self.primary.encode()?;
        let secondary = self
            .secondary
            .encode()
            .map_err(VersionedTwoVcpuFullControllerCheckpointError::Secondary)?;
        let total_len = TWO_VCPU_FULL_CONTROLLER_HEADER_LEN
            .checked_add(primary.len())
            .and_then(|length| length.checked_add(secondary.len()))
            .and_then(|length| length.checked_add(TWO_VCPU_FULL_CONTROLLER_STATE_LEN))
            .ok_or(VersionedTwoVcpuFullControllerCheckpointError::LengthOverflow)?;

        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_TWO_VCPU_FULL_CONTROLLER_MAGIC);
        bytes.extend_from_slice(&VERSIONED_TWO_VCPU_FULL_CONTROLLER_VERSION.to_le_bytes());
        bytes.extend_from_slice(&VERSIONED_TWO_VCPU_FULL_CONTROLLER_ARCH_X86_64.to_le_bytes());
        bytes.extend_from_slice(&(TWO_VCPU_FULL_CONTROLLER_HEADER_LEN as u32).to_le_bytes());
        bytes.extend_from_slice(&(total_len as u64).to_le_bytes());
        bytes.extend_from_slice(&(primary.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(secondary.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&self.vcpu_ids[0].get().to_le_bytes());
        bytes.extend_from_slice(&self.vcpu_ids[1].get().to_le_bytes());
        bytes.extend_from_slice(&self.mp_states[0].to_le_bytes());
        bytes.extend_from_slice(&self.mp_states[1].to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());

        bytes.extend_from_slice(&primary);
        bytes.extend_from_slice(&secondary);
        bytes.extend_from_slice(&self.master_pic.semantic_bytes());
        bytes.extend_from_slice(&self.slave_pic.semantic_bytes());
        bytes.extend_from_slice(&self.ioapic.semantic_bytes());
        bytes.extend_from_slice(&self.lapics[0].regs);
        bytes.extend_from_slice(&self.lapics[1].regs);
        debug_assert_eq!(bytes.len(), total_len);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, VersionedTwoVcpuFullControllerCheckpointError> {
        if bytes.len() < TWO_VCPU_FULL_CONTROLLER_HEADER_LEN {
            return Err(
                VersionedTwoVcpuFullControllerCheckpointError::InvalidTotalLength {
                    declared: 0,
                    actual: bytes.len(),
                },
            );
        }
        if bytes[0..8] != VERSIONED_TWO_VCPU_FULL_CONTROLLER_MAGIC {
            return Err(VersionedTwoVcpuFullControllerCheckpointError::InvalidMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("fixed version field"));
        if version != VERSIONED_TWO_VCPU_FULL_CONTROLLER_VERSION {
            return Err(VersionedTwoVcpuFullControllerCheckpointError::UnsupportedVersion(version));
        }
        let architecture =
            u16::from_le_bytes(bytes[10..12].try_into().expect("fixed architecture field"));
        if architecture != VERSIONED_TWO_VCPU_FULL_CONTROLLER_ARCH_X86_64 {
            return Err(
                VersionedTwoVcpuFullControllerCheckpointError::UnsupportedArchitecture(
                    architecture,
                ),
            );
        }
        let header_len =
            u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header length field"));
        if header_len != TWO_VCPU_FULL_CONTROLLER_HEADER_LEN as u32 {
            return Err(
                VersionedTwoVcpuFullControllerCheckpointError::InvalidHeaderLength(header_len),
            );
        }
        let declared_total =
            u64::from_le_bytes(bytes[16..24].try_into().expect("fixed total length field"));
        if usize::try_from(declared_total).ok() != Some(bytes.len()) {
            return Err(
                VersionedTwoVcpuFullControllerCheckpointError::InvalidTotalLength {
                    declared: declared_total,
                    actual: bytes.len(),
                },
            );
        }
        let primary_len = u64::from_le_bytes(
            bytes[24..32]
                .try_into()
                .expect("fixed primary length field"),
        );
        let secondary_len = u64::from_le_bytes(
            bytes[32..40]
                .try_into()
                .expect("fixed secondary length field"),
        );
        let ids = [
            VcpuId::new(u16::from_le_bytes(
                bytes[40..42].try_into().expect("fixed primary vCPU id"),
            )),
            VcpuId::new(u16::from_le_bytes(
                bytes[42..44].try_into().expect("fixed secondary vCPU id"),
            )),
        ];
        validate_vcpu_ids(ids)?;
        let mp_states = [
            u32::from_le_bytes(bytes[44..48].try_into().expect("fixed primary MP state")),
            u32::from_le_bytes(bytes[48..52].try_into().expect("fixed secondary MP state")),
        ];
        validate_mp_states(ids, mp_states)?;
        let flags = u32::from_le_bytes(bytes[52..56].try_into().expect("fixed flags field"));
        if flags != 0 {
            return Err(VersionedTwoVcpuFullControllerCheckpointError::NonZeroFlags(
                flags,
            ));
        }
        let reserved = u64::from_le_bytes(bytes[56..64].try_into().expect("fixed reserved field"));
        if reserved != 0 {
            return Err(VersionedTwoVcpuFullControllerCheckpointError::NonZeroReserved(reserved));
        }

        let primary_len = usize::try_from(primary_len).map_err(|_| {
            VersionedTwoVcpuFullControllerCheckpointError::InvalidPrimaryLength(primary_len)
        })?;
        if primary_len == 0 {
            return Err(VersionedTwoVcpuFullControllerCheckpointError::InvalidPrimaryLength(0));
        }
        let secondary_len = usize::try_from(secondary_len).map_err(|_| {
            VersionedTwoVcpuFullControllerCheckpointError::InvalidSecondaryLength(secondary_len)
        })?;
        if secondary_len == 0 {
            return Err(VersionedTwoVcpuFullControllerCheckpointError::InvalidSecondaryLength(0));
        }
        let expected_len = TWO_VCPU_FULL_CONTROLLER_HEADER_LEN
            .checked_add(primary_len)
            .and_then(|length| length.checked_add(secondary_len))
            .and_then(|length| length.checked_add(TWO_VCPU_FULL_CONTROLLER_STATE_LEN))
            .ok_or(VersionedTwoVcpuFullControllerCheckpointError::LengthOverflow)?;
        if expected_len != bytes.len() {
            return Err(
                VersionedTwoVcpuFullControllerCheckpointError::InvalidTotalLength {
                    declared: declared_total,
                    actual: expected_len,
                },
            );
        }

        let primary_start = TWO_VCPU_FULL_CONTROLLER_HEADER_LEN;
        let primary_end = primary_start + primary_len;
        let secondary_end = primary_end + secondary_len;
        let primary = VersionedPageVcpuCheckpointV1::decode(&bytes[primary_start..primary_end])?;
        let secondary = VersionedVcpuStateSnapshotV1::decode(&bytes[primary_end..secondary_end])
            .map_err(VersionedTwoVcpuFullControllerCheckpointError::Secondary)?;

        let mut offset = secondary_end;
        let master_end = offset + KVM_PIC_STATE_SIZE;
        let master_pic = MasterPicStateSnapshot::from_semantic_bytes(
            bytes[offset..master_end]
                .try_into()
                .expect("validated master PIC semantic length"),
        );
        offset = master_end;
        let slave_end = offset + KVM_PIC_STATE_SIZE;
        let slave_pic = SlavePicStateSnapshot::from_semantic_bytes(
            bytes[offset..slave_end]
                .try_into()
                .expect("validated slave PIC semantic length"),
        );
        offset = slave_end;
        let ioapic_end = offset + KVM_IOAPIC_STATE_SIZE;
        let ioapic = IoapicStateSnapshot::from_semantic_bytes(
            bytes[offset..ioapic_end]
                .try_into()
                .expect("validated IOAPIC semantic length"),
        );
        if ioapic.pad() != 0 {
            return Err(
                VersionedTwoVcpuFullControllerCheckpointError::NonZeroIoapicPad(ioapic.pad()),
            );
        }
        offset = ioapic_end;
        let first_lapic_end = offset + KVM_APIC_REG_SIZE;
        let first_lapic = KvmLapicState {
            regs: bytes[offset..first_lapic_end]
                .try_into()
                .expect("validated first LAPIC length"),
        };
        offset = first_lapic_end;
        let second_lapic_end = offset + KVM_APIC_REG_SIZE;
        let second_lapic = KvmLapicState {
            regs: bytes[offset..second_lapic_end]
                .try_into()
                .expect("validated second LAPIC length"),
        };
        debug_assert_eq!(second_lapic_end, bytes.len());

        Ok(Self {
            primary,
            secondary,
            vcpu_ids: ids,
            mp_states,
            master_pic,
            slave_pic,
            ioapic,
            lapics: [first_lapic, second_lapic],
        })
    }

    pub fn materialize(
        &self,
        host_msrs: &HostMsrIndexList,
    ) -> Result<BoundedTwoVcpuFullControllerCheckpoint, VersionedTwoVcpuFullControllerCheckpointError>
    {
        validate_vcpu_ids(self.vcpu_ids)?;
        validate_mp_states(self.vcpu_ids, self.mp_states)?;
        if self.ioapic.pad() != 0 {
            return Err(VersionedTwoVcpuFullControllerCheckpointError::NonZeroIoapicPad(
                self.ioapic.pad(),
            ));
        }
        let primary = self.primary.materialize(host_msrs)?;
        let secondary = self
            .secondary
            .materialize(host_msrs)
            .map_err(VersionedTwoVcpuFullControllerCheckpointError::Secondary)?;
        let base = BoundedTwoVcpuCheckpoint {
            primary_id: self.vcpu_ids[0],
            primary,
            secondary_id: self.vcpu_ids[1],
            secondary,
        };
        Ok(BoundedTwoVcpuFullControllerCheckpoint {
            base,
            master_pic: self.master_pic.clone(),
            slave_pic: self.slave_pic.clone(),
            ioapic: self.ioapic.clone(),
            lapics: [
                (self.vcpu_ids[0], self.lapics[0].clone()),
                (self.vcpu_ids[1], self.lapics[1].clone()),
            ],
            mp_states: [
                (self.vcpu_ids[0], self.mp_states[0]),
                (self.vcpu_ids[1], self.mp_states[1]),
            ],
        })
    }
}

fn validate_vcpu_ids(
    ids: [VcpuId; 2],
) -> Result<(), VersionedTwoVcpuFullControllerCheckpointError> {
    if ids[0].get() >= ids[1].get() {
        return Err(
            VersionedTwoVcpuFullControllerCheckpointError::NonCanonicalVcpuIds {
                primary: ids[0].get(),
                secondary: ids[1].get(),
            },
        );
    }
    Ok(())
}

fn validate_mp_states(
    ids: [VcpuId; 2],
    states: [u32; 2],
) -> Result<(), VersionedTwoVcpuFullControllerCheckpointError> {
    for (id, state) in ids.into_iter().zip(states) {
        if state > X86_MP_STATE_MAX {
            return Err(
                VersionedTwoVcpuFullControllerCheckpointError::InvalidMpState {
                    vcpu: id.get(),
                    state,
                },
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod two_vcpu_full_controller_schema_tests {
    use super::*;

    #[test]
    fn vcpu_ids_must_be_canonical() {
        assert!(validate_vcpu_ids([VcpuId::new(0), VcpuId::new(1)]).is_ok());
        assert!(matches!(
            validate_vcpu_ids([VcpuId::new(1), VcpuId::new(1)]),
            Err(VersionedTwoVcpuFullControllerCheckpointError::NonCanonicalVcpuIds { .. })
        ));
        assert!(matches!(
            validate_vcpu_ids([VcpuId::new(2), VcpuId::new(1)]),
            Err(VersionedTwoVcpuFullControllerCheckpointError::NonCanonicalVcpuIds { .. })
        ));
    }

    #[test]
    fn x86_mp_states_are_bounded() {
        assert!(validate_mp_states([VcpuId::new(0), VcpuId::new(1)], [0, 4]).is_ok());
        assert_eq!(
            validate_mp_states([VcpuId::new(0), VcpuId::new(1)], [0, 5]),
            Err(
                VersionedTwoVcpuFullControllerCheckpointError::InvalidMpState { vcpu: 1, state: 5 }
            )
        );
    }
}
