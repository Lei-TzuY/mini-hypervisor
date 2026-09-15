use super::{
    BoundedCheckpointPage, BoundedVcpuPageSetCheckpoint, BOUNDED_CHECKPOINT_PAGE_SET_LIMIT,
};
use crate::kvm::msr::value_set::{GuestMsrSnapshot, GuestMsrValueSet};
use crate::kvm::msr::{GuestMsrAccessPolicy, HostMsrIndexList, MsrIndex};
use crate::kvm::sys;
use crate::long_mode::LONG_MODE_PAGE_SIZE;
use crate::memory::GuestPhysAddr;
use crate::state_snapshot::VcpuStateSnapshot;
use crate::vcpu::{
    VcpuDescriptorTableState, VcpuRegisterSnapshot, VcpuSegmentState, VcpuSpecialRegisterSnapshot,
};
use std::fmt;

pub const VERSIONED_PAGE_VCPU_CHECKPOINT_MAGIC: [u8; 8] = *b"MHVCKPT\0";
pub const VERSIONED_PAGE_VCPU_CHECKPOINT_VERSION: u16 = 1;
pub const VERSIONED_PAGE_VCPU_CHECKPOINT_ARCH_X86_64: u16 = 1;
pub const VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT: usize = 256;

const HEADER_LEN: usize = 40;
const REGISTER_BLOCK_LEN: usize = 18 * 8;
const SEGMENT_BLOCK_LEN: usize = 24;
const DTABLE_BLOCK_LEN: usize = 16;
const SPECIAL_REGISTER_BLOCK_LEN: usize = 8 * SEGMENT_BLOCK_LEN + 2 * DTABLE_BLOCK_LEN + 11 * 8;
const PAGE_RECORD_LEN: usize = 8 + LONG_MODE_PAGE_SIZE as usize;
const MSR_RECORD_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionedPageVcpuCheckpointError {
    InvalidMagic,
    UnsupportedVersion(u16),
    UnsupportedArchitecture(u16),
    InvalidHeaderLength(u32),
    InvalidTotalLength {
        declared: u64,
        actual: usize,
    },
    InvalidPageSize(u32),
    InvalidPageCount(u16),
    InvalidMsrCount(u16),
    NonZeroFlags(u32),
    NonZeroReserved {
        field: &'static str,
        value: u64,
    },
    LengthOverflow,
    Truncated {
        offset: usize,
        needed: usize,
        remaining: usize,
    },
    UnalignedPage(u64),
    PageAddressOverflow(u64),
    NonCanonicalPageOrder {
        previous: u64,
        current: u64,
    },
    InvalidPageLength {
        address: u64,
        length: usize,
    },
    InvalidSegmentField {
        segment: u8,
        field: &'static str,
        value: u8,
    },
    NonCanonicalMsrOrder {
        previous: u32,
        current: u32,
    },
    HostMsrIncompatible(String),
    MsrValueSet(String),
    MsrSnapshot(String),
}

impl fmt::Display for VersionedPageVcpuCheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "checkpoint schema magic does not match MHVCKPT"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported checkpoint schema version {version}")
            }
            Self::UnsupportedArchitecture(architecture) => write!(
                f,
                "unsupported checkpoint schema architecture identifier {architecture}"
            ),
            Self::InvalidHeaderLength(length) => {
                write!(f, "checkpoint schema header length {length} is not {HEADER_LEN}")
            }
            Self::InvalidTotalLength { declared, actual } => write!(
                f,
                "checkpoint schema declares total length {declared}, actual byte length is {actual}"
            ),
            Self::InvalidPageSize(size) => write!(
                f,
                "checkpoint schema page size {size} is not {LONG_MODE_PAGE_SIZE}"
            ),
            Self::InvalidPageCount(count) => write!(
                f,
                "checkpoint schema page count {count} is outside 1..={BOUNDED_CHECKPOINT_PAGE_SET_LIMIT}"
            ),
            Self::InvalidMsrCount(count) => write!(
                f,
                "checkpoint schema MSR count {count} exceeds bounded limit {VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT}"
            ),
            Self::NonZeroFlags(flags) => {
                write!(f, "checkpoint schema v1 flags must be zero, got {flags:#x}")
            }
            Self::NonZeroReserved { field, value } => write!(
                f,
                "checkpoint schema reserved field {field} must be zero, got {value:#x}"
            ),
            Self::LengthOverflow => write!(f, "checkpoint schema length arithmetic overflowed"),
            Self::Truncated {
                offset,
                needed,
                remaining,
            } => write!(
                f,
                "checkpoint schema is truncated at offset {offset}: need {needed} bytes, only {remaining} remain"
            ),
            Self::UnalignedPage(address) => write!(
                f,
                "checkpoint schema page {address:#x} is not {LONG_MODE_PAGE_SIZE}-byte aligned"
            ),
            Self::PageAddressOverflow(address) => write!(
                f,
                "checkpoint schema page {address:#x} overflows the guest physical address space"
            ),
            Self::NonCanonicalPageOrder { previous, current } => write!(
                f,
                "checkpoint schema pages are not strictly increasing: {previous:#x} then {current:#x}"
            ),
            Self::InvalidPageLength { address, length } => write!(
                f,
                "checkpoint page {address:#x} has {length} bytes instead of {LONG_MODE_PAGE_SIZE}"
            ),
            Self::InvalidSegmentField {
                segment,
                field,
                value,
            } => write!(
                f,
                "checkpoint segment {segment} has invalid {field} value {value:#x}"
            ),
            Self::NonCanonicalMsrOrder { previous, current } => write!(
                f,
                "checkpoint MSRs are not strictly increasing: {previous:#x} then {current:#x}"
            ),
            Self::HostMsrIncompatible(detail) => {
                write!(f, "checkpoint MSR policy is incompatible with this host: {detail}")
            }
            Self::MsrValueSet(detail) => {
                write!(f, "checkpoint MSR value set is invalid: {detail}")
            }
            Self::MsrSnapshot(detail) => {
                write!(f, "checkpoint MSR snapshot is invalid: {detail}")
            }
        }
    }
}

impl std::error::Error for VersionedPageVcpuCheckpointError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedPageVcpuCheckpointV1 {
    pages: Vec<BoundedCheckpointPage>,
    registers: VcpuRegisterSnapshot,
    special_registers: VcpuSpecialRegisterSnapshot,
    msrs: Vec<(MsrIndex, u64)>,
}

impl VersionedPageVcpuCheckpointV1 {
    pub fn from_checkpoint(
        checkpoint: &BoundedVcpuPageSetCheckpoint,
    ) -> Result<Self, VersionedPageVcpuCheckpointError> {
        validate_pages(checkpoint.pages())?;
        let mut msrs = checkpoint
            .vcpu()
            .msrs()
            .values()
            .values()
            .iter()
            .map(|value| (value.index(), value.value()))
            .collect::<Vec<_>>();
        if msrs.len() > VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT {
            return Err(VersionedPageVcpuCheckpointError::InvalidMsrCount(
                u16::try_from(msrs.len()).unwrap_or(u16::MAX),
            ));
        }
        msrs.sort_unstable_by_key(|(index, _)| index.get());
        validate_msr_order(&msrs)?;
        Ok(Self {
            pages: checkpoint.pages().to_vec(),
            registers: *checkpoint.vcpu().registers(),
            special_registers: *checkpoint.vcpu().special_registers(),
            msrs,
        })
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        VERSIONED_PAGE_VCPU_CHECKPOINT_VERSION
    }

    #[must_use]
    pub fn pages(¶»§q«^