use super::{BoundedCheckpointPage, BoundedVcpuPageSetCheckpoint, BOUNDED_CHECKPOINT_PAGE_SET_LIMIT};
use crate::kvm::msr::value_set::{GuestMsrSnapshot, GuestMsrValueSet};
use crate::kvm::msr::{GuestMsrAccessPolicy, HostMsrIndexList, MsrIndex};
use crate::kvm::sys;
use crate::long_mode::LONG_MODE_PAGE_SIZE;
use crate::memory::GuestPhysAddr;
use crate::vcpu::{
    VcpuDescriptorTableState, VcpuRegisterSnapshot, VcpuSegmentState,
    VcpuSpecialRegisterSnapshot,
};
use crate::VcpuStateSnapshot;
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
    InvalidTotalLength { declared: u64, actual: usize },
    InvalidPageSize(u32),
    InvalidPageCount(u16),
    InvalidMsrCount(u16),
    NonZeroFlags(u32),
    NonZeroReserved { field: &'static str, value: u64 },
    LengthOverflow,
    Truncated {
        offset: usize,
        needed: usize,
        remaining: usize,
    },
    UnalignedPage(u64),
    PageAddressOverflow(u64),
    NonCanonicalPageOrder { previous: u64, current: u64 },
    InvalidPageLength { address: u64, length: usize },
    InvalidSegmentField {
        segment: u8,
        field: &'static str,
        value: u8,
    },
    NonCanonicalMsrOrder { previous: u32, current: u32 },
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
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        &self.pages
    }

    #[must_use]
    pub fn msr_count(&self) -> usize {
        self.msrs.len()
    }

    pub fn encode(&self) -> Result<Vec<u8>, VersionedPageVcpuCheckpointError> {
        validate_pages(&self.pages)?;
        validate_msr_order(&self.msrs)?;
        if self.msrs.len() > VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT {
            return Err(VersionedPageVcpuCheckpointError::InvalidMsrCount(
                u16::try_from(self.msrs.len()).unwrap_or(u16::MAX),
            ));
        }
        let total_len = encoded_length(self.pages.len(), self.msrs.len())?;
        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&VERSIONED_PAGE_VCPU_CHECKPOINT_MAGIC);
        push_u16(&mut bytes, VERSIONED_PAGE_VCPU_CHECKPOINT_VERSION);
        push_u16(&mut bytes, VERSIONED_PAGE_VCPU_CHECKPOINT_ARCH_X86_64);
        push_u32(&mut bytes, HEADER_LEN as u32);
        push_u64(&mut bytes, total_len as u64);
        push_u32(&mut bytes, LONG_MODE_PAGE_SIZE as u32);
        push_u16(
            &mut bytes,
            u16::try_from(self.pages.len()).expect("bounded checkpoint page count fits u16"),
        );
        push_u16(
            &mut bytes,
            u16::try_from(self.msrs.len()).expect("bounded checkpoint MSR count fits u16"),
        );
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);

        for page in &self.pages {
            push_u64(&mut bytes, page.address().get());
            bytes.extend_from_slice(page.bytes());
        }
        encode_registers(&mut bytes, &self.registers);
        encode_special_registers(&mut bytes, &self.special_registers);
        for (index, value) in &self.msrs {
            push_u32(&mut bytes, index.get());
            push_u32(&mut bytes, 0);
            push_u64(&mut bytes, *value);
        }
        debug_assert_eq!(bytes.len(), total_len);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, VersionedPageVcpuCheckpointError> {
        let mut cursor = Cursor::new(bytes);
        let magic = cursor.take(8)?;
        if magic != VERSIONED_PAGE_VCPU_CHECKPOINT_MAGIC {
            return Err(VersionedPageVcpuCheckpointError::InvalidMagic);
        }
        let version = cursor.u16()?;
        if version != VERSIONED_PAGE_VCPU_CHECKPOINT_VERSION {
            return Err(VersionedPageVcpuCheckpointError::UnsupportedVersion(version));
        }
        let architecture = cursor.u16()?;
        if architecture != VERSIONED_PAGE_VCPU_CHECKPOINT_ARCH_X86_64 {
            return Err(VersionedPageVcpuCheckpointError::UnsupportedArchitecture(
                architecture,
            ));
        }
        let header_len = cursor.u32()?;
        if header_len != HEADER_LEN as u32 {
            return Err(VersionedPageVcpuCheckpointError::InvalidHeaderLength(
                header_len,
            ));
        }
        let declared_total = cursor.u64()?;
        if declared_total != bytes.len() as u64 {
            return Err(VersionedPageVcpuCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: bytes.len(),
            });
        }
        let page_size = cursor.u32()?;
        if page_size != LONG_MODE_PAGE_SIZE as u32 {
            return Err(VersionedPageVcpuCheckpointError::InvalidPageSize(page_size));
        }
        let page_count = cursor.u16()?;
        if page_count == 0 || usize::from(page_count) > BOUNDED_CHECKPOINT_PAGE_SET_LIMIT {
            return Err(VersionedPageVcpuCheckpointError::InvalidPageCount(page_count));
        }
        let msr_count = cursor.u16()?;
        if usize::from(msr_count) > VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT {
            return Err(VersionedPageVcpuCheckpointError::InvalidMsrCount(msr_count));
        }
        let flags = cursor.u32()?;
        if flags != 0 {
            return Err(VersionedPageVcpuCheckpointError::NonZeroFlags(flags));
        }
        let reserved = cursor.u32()?;
        if reserved != 0 {
            return Err(VersionedPageVcpuCheckpointError::NonZeroReserved {
                field: "header.reserved",
                value: u64::from(reserved),
            });
        }
        let expected_len = encoded_length(usize::from(page_count), usize::from(msr_count))?;
        if expected_len != bytes.len() {
            return Err(VersionedPageVcpuCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: expected_len,
            });
        }

        let mut pages = Vec::with_capacity(usize::from(page_count));
        let mut previous_page = None;
        for _ in 0..page_count {
            let address = cursor.u64()?;
            validate_page_address(address)?;
            if let Some(previous) = previous_page {
                if address <= previous {
                    return Err(VersionedPageVcpuCheckpointError::NonCanonicalPageOrder {
                        previous,
                        current: address,
                    });
                }
            }
            let page_bytes = cursor.take(LONG_MODE_PAGE_SIZE as usize)?.to_vec();
            pages.push(BoundedCheckpointPage {
                address: GuestPhysAddr::new(address),
                bytes: page_bytes,
            });
            previous_page = Some(address);
        }

        let registers = VcpuRegisterSnapshot::from_kvm_regs(decode_registers(&mut cursor)?);
        let special_registers =
            VcpuSpecialRegisterSnapshot::from_kvm_sregs(decode_special_registers(&mut cursor)?);
        let mut msrs = Vec::with_capacity(usize::from(msr_count));
        let mut previous_msr = None;
        for _ in 0..msr_count {
            let index = cursor.u32()?;
            let reserved = cursor.u32()?;
            if reserved != 0 {
                return Err(VersionedPageVcpuCheckpointError::NonZeroReserved {
                    field: "msr.reserved",
                    value: u64::from(reserved),
                });
            }
            if let Some(previous) = previous_msr {
                if index <= previous {
                    return Err(VersionedPageVcpuCheckpointError::NonCanonicalMsrOrder {
                        previous,
                        current: index,
                    });
                }
            }
            let value = cursor.u64()?;
            msrs.push((MsrIndex::new(index), value));
            previous_msr = Some(index);
        }
        if cursor.remaining() != 0 {
            return Err(VersionedPageVcpuCheckpointError::InvalidTotalLength {
                declared: declared_total,
                actual: cursor.offset,
            });
        }
        Ok(Self {
            pages,
            registers,
            special_registers,
            msrs,
        })
    }

    pub fn materialize(
        &self,
        host_msrs: &HostMsrIndexList,
    ) -> Result<BoundedVcpuPageSetCheckpoint, VersionedPageVcpuCheckpointError> {
        validate_pages(&self.pages)?;
        validate_msr_order(&self.msrs)?;
        let indices = self.msrs.iter().map(|(index, _)| *index).collect::<Vec<_>>();
        let policy = GuestMsrAccessPolicy::from_host(host_msrs, &indices).map_err(|error| {
            VersionedPageVcpuCheckpointError::HostMsrIncompatible(error.to_string())
        })?;
        let values = GuestMsrValueSet::from_policy(&policy, &self.msrs)
            .map_err(|error| VersionedPageVcpuCheckpointError::MsrValueSet(error.to_string()))?;
        let msrs = GuestMsrSnapshot::from_capture(&policy, &values)
            .map_err(|error| VersionedPageVcpuCheckpointError::MsrSnapshot(error.to_string()))?;
        let vcpu = VcpuStateSnapshot {
            registers: self.registers,
            special_registers: self.special_registers,
            msrs,
        };
        Ok(BoundedVcpuPageSetCheckpoint {
            pages: self.pages.clone(),
            vcpu,
        })
    }
}

fn encoded_length(
    page_count: usize,
    msr_count: usize,
) -> Result<usize, VersionedPageVcpuCheckpointError> {
    let pages = PAGE_RECORD_LEN
        .checked_mul(page_count)
        .ok_or(VersionedPageVcpuCheckpointError::LengthOverflow)?;
    let msrs = MSR_RECORD_LEN
        .checked_mul(msr_count)
        .ok_or(VersionedPageVcpuCheckpointError::LengthOverflow)?;
    HEADER_LEN
        .checked_add(pages)
        .and_then(|length| length.checked_add(REGISTER_BLOCK_LEN))
        .and_then(|length| length.checked_add(SPECIAL_REGISTER_BLOCK_LEN))
        .and_then(|length| length.checked_add(msrs))
        .ok_or(VersionedPageVcpuCheckpointError::LengthOverflow)
}

fn validate_pages(pages: &[BoundedCheckpointPage]) -> Result<(), VersionedPageVcpuCheckpointError> {
    if pages.is_empty() || pages.len() > BOUNDED_CHECKPOINT_PAGE_SET_LIMIT {
        return Err(VersionedPageVcpuCheckpointError::InvalidPageCount(
            u16::try_from(pages.len()).unwrap_or(u16::MAX),
        ));
    }
    let mut previous = None;
    for page in pages {
        let address = page.address().get();
        validate_page_address(address)?;
        if page.bytes().len() != LONG_MODE_PAGE_SIZE as usize {
            return Err(VersionedPageVcpuCheckpointError::InvalidPageLength {
                address,
                length: page.bytes().len(),
            });
        }
        if let Some(previous) = previous {
            if address <= previous {
                return Err(VersionedPageVcpuCheckpointError::NonCanonicalPageOrder {
                    previous,
                    current: address,
                });
            }
        }
        previous = Some(address);
    }
    Ok(())
}

fn validate_page_address(address: u64) -> Result<(), VersionedPageVcpuCheckpointError> {
    if address % LONG_MODE_PAGE_SIZE != 0 {
        return Err(VersionedPageVcpuCheckpointError::UnalignedPage(address));
    }
    address
        .checked_add(LONG_MODE_PAGE_SIZE)
        .ok_or(VersionedPageVcpuCheckpointError::PageAddressOverflow(address))?;
    Ok(())
}

fn validate_msr_order(msrs: &[(MsrIndex, u64)]) -> Result<(), VersionedPageVcpuCheckpointError> {
    let mut previous = None;
    for (index, _) in msrs {
        let current = index.get();
        if let Some(previous) = previous {
            if current <= previous {
                return Err(VersionedPageVcpuCheckpointError::NonCanonicalMsrOrder {
                    previous,
                    current,
                });
            }
        }
        previous = Some(current);
    }
    Ok(())
}

fn encode_registers(bytes: &mut Vec<u8>, registers: &VcpuRegisterSnapshot) {
    for value in [
        registers.rax(),
        registers.rbx(),
        registers.rcx(),
        registers.rdx(),
        registers.rsi(),
        registers.rdi(),
        registers.rsp(),
        registers.rbp(),
        registers.r8(),
        registers.r9(),
        registers.r10(),
        registers.r11(),
        registers.r12(),
        registers.r13(),
        registers.r14(),
        registers.r15(),
        registers.rip(),
        registers.rflags(),
    ] {
        push_u64(bytes, value);
    }
}

fn decode_registers(
    cursor: &mut Cursor<'_>,
) -> Result<sys::KvmRegs, VersionedPageVcpuCheckpointError> {
    Ok(sys::KvmRegs {
        rax: cursor.u64()?,
        rbx: cursor.u64()?,
        rcx: cursor.u64()?,
        rdx: cursor.u64()?,
        rsi: cursor.u64()?,
        rdi: cursor.u64()?,
        rsp: cursor.u64()?,
        rbp: cursor.u64()?,
        r8: cursor.u64()?,
        r9: cursor.u64()?,
        r10: cursor.u64()?,
        r11: cursor.u64()?,
        r12: cursor.u64()?,
        r13: cursor.u64()?,
        r14: cursor.u64()?,
        r15: cursor.u64()?,
        rip: cursor.u64()?,
        rflags: cursor.u64()?,
    })
}

fn encode_special_registers(bytes: &mut Vec<u8>, sregs: &VcpuSpecialRegisterSnapshot) {
    for segment in [
        sregs.cs(),
        sregs.ds(),
        sregs.es(),
        sregs.fs(),
        sregs.gs(),
        sregs.ss(),
        sregs.tr(),
        sregs.ldt(),
    ] {
        encode_segment(bytes, segment);
    }
    encode_dtable(bytes, sregs.gdt());
    encode_dtable(bytes, sregs.idt());
    for value in [
        sregs.cr0(),
        sregs.cr2(),
        sregs.cr3(),
        sregs.cr4(),
        sregs.cr8(),
        sregs.efer(),
        sregs.apic_base(),
        sregs.interrupt_bitmap()[0],
        sregs.interrupt_bitmap()[1],
        sregs.interrupt_bitmap()[2],
        sregs.interrupt_bitmap()[3],
    ] {
        push_u64(bytes, value);
    }
}

fn decode_special_registers(
    cursor: &mut Cursor<'_>,
) -> Result<sys::KvmSregs, VersionedPageVcpuCheckpointError> {
    let mut segments = [sys::KvmSegment::default(); 8];
    for (index, segment) in segments.iter_mut().enumerate() {
        *segment = decode_segment(cursor, index as u8)?;
    }
    let gdt = decode_dtable(cursor, "gdt.reserved")?;
    let idt = decode_dtable(cursor, "idt.reserved")?;
    Ok(sys::KvmSregs {
        cs: segments[0],
        ds: segments[1],
        es: segments[2],
        fs: segments[3],
        gs: segments[4],
        ss: segments[5],
        tr: segments[6],
        ldt: segments[7],
        gdt,
        idt,
        cr0: cursor.u64()?,
        cr2: cursor.u64()?,
        cr3: cursor.u64()?,
        cr4: cursor.u64()?,
        cr8: cursor.u64()?,
        efer: cursor.u64()?,
        apic_base: cursor.u64()?,
        interrupt_bitmap: [
            cursor.u64()?,
            cursor.u64()?,
            cursor.u64()?,
            cursor.u64()?,
        ],
    })
}

fn encode_segment(bytes: &mut Vec<u8>, segment: VcpuSegmentState) {
    push_u64(bytes, segment.base());
    push_u32(bytes, segment.limit());
    push_u16(bytes, segment.selector());
    bytes.extend_from_slice(&[
        segment.segment_type(),
        segment.present(),
        segment.dpl(),
        segment.db(),
        segment.s(),
        segment.l(),
        segment.g(),
        segment.avl(),
        segment.unusable(),
        0,
    ]);
}

fn decode_segment(
    cursor: &mut Cursor<'_>,
    segment: u8,
) -> Result<sys::KvmSegment, VersionedPageVcpuCheckpointError> {
    let base = cursor.u64()?;
    let limit = cursor.u32()?;
    let selector = cursor.u16()?;
    let type_ = cursor.u8()?;
    let present = cursor.u8()?;
    let dpl = cursor.u8()?;
    let db = cursor.u8()?;
    let s = cursor.u8()?;
    let l = cursor.u8()?;
    let g = cursor.u8()?;
    let avl = cursor.u8()?;
    let unusable = cursor.u8()?;
    let reserved = cursor.u8()?;
    if type_ > 0x0f {
        return Err(VersionedPageVcpuCheckpointError::InvalidSegmentField {
            segment,
            field: "type",
            value: type_,
        });
    }
    if dpl > 3 {
        return Err(VersionedPageVcpuCheckpointError::InvalidSegmentField {
            segment,
            field: "dpl",
            value: dpl,
        });
    }
    for (field, value) in [
        ("present", present),
        ("db", db),
        ("s", s),
        ("l", l),
        ("g", g),
        ("avl", avl),
        ("unusable", unusable),
    ] {
        if value > 1 {
            return Err(VersionedPageVcpuCheckpointError::InvalidSegmentField {
                segment,
                field,
                value,
            });
        }
    }
    if reserved != 0 {
        return Err(VersionedPageVcpuCheckpointError::NonZeroReserved {
            field: "segment.reserved",
            value: u64::from(reserved),
        });
    }
    Ok(sys::KvmSegment {
        base,
        limit,
        selector,
        type_,
        present,
        dpl,
        db,
        s,
        l,
        g,
        avl,
        unusable,
        padding: 0,
    })
}

fn encode_dtable(bytes: &mut Vec<u8>, table: VcpuDescriptorTableState) {
    push_u64(bytes, table.base());
    push_u16(bytes, table.limit());
    bytes.extend_from_slice(&[0; 6]);
}

fn decode_dtable(
    cursor: &mut Cursor<'_>,
    reserved_field: &'static str,
) -> Result<sys::KvmDtable, VersionedPageVcpuCheckpointError> {
    let base = cursor.u64()?;
    let limit = cursor.u16()?;
    let reserved = cursor.take(6)?;
    if reserved.iter().any(|byte| *byte != 0) {
        let value = reserved
            .iter()
            .enumerate()
            .fold(0_u64, |acc, (index, byte)| acc | (u64::from(*byte) << (8 * index)));
        return Err(VersionedPageVcpuCheckpointError::NonZeroReserved {
            field: reserved_field,
            value,
        });
    }
    Ok(sys::KvmDtable {
        base,
        limit,
        padding: [0; 3],
    })
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], VersionedPageVcpuCheckpointError> {
        let remaining = self.remaining();
        if length > remaining {
            return Err(VersionedPageVcpuCheckpointError::Truncated {
                offset: self.offset,
                needed: length,
                remaining,
            });
        }
        let start = self.offset;
        self.offset += length;
        Ok(&self.bytes[start..self.offset])
    }

    fn u8(&mut self) -> Result<u8, VersionedPageVcpuCheckpointError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, VersionedPageVcpuCheckpointError> {
        let bytes: [u8; 2] = self.take(2)?.try_into().expect("cursor returned two bytes");
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, VersionedPageVcpuCheckpointError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().expect("cursor returned four bytes");
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, VersionedPageVcpuCheckpointError> {
        let bytes: [u8; 8] = self.take(8)?.try_into().expect("cursor returned eight bytes");
        Ok(u64::from_le_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(seed: u8) -> sys::KvmSegment {
        sys::KvmSegment {
            base: u64::from(seed) << 32,
            limit: 0xffff,
            selector: u16::from(seed) << 3,
            type_: seed & 0x0f,
            present: 1,
            dpl: seed & 0x03,
            db: 0,
            s: 1,
            l: 1,
            g: 1,
            avl: 0,
            unusable: 0,
            padding: 0,
        }
    }

    fn checkpoint() -> (BoundedVcpuPageSetCheckpoint, HostMsrIndexList) {
        let registers = VcpuRegisterSnapshot::from_kvm_regs(sys::KvmRegs {
            rax: 1,
            rbx: 2,
            rcx: 3,
            rdx: 4,
            rsi: 5,
            rdi: 6,
            rsp: 7,
            rbp: 8,
            r8: 9,
            r9: 10,
            r10: 11,
            r11: 12,
            r12: 13,
            r13: 14,
            r14: 15,
            r15: 16,
            rip: 17,
            rflags: 0x202,
        });
        let special_registers = VcpuSpecialRegisterSnapshot::from_kvm_sregs(sys::KvmSregs {
            cs: segment(1),
            ds: segment(2),
            es: segment(3),
            fs: segment(4),
            gs: segment(5),
            ss: segment(6),
            tr: segment(7),
            ldt: segment(8),
            gdt: sys::KvmDtable {
                base: 0x5000,
                limit: 0x17,
                padding: [0; 3],
            },
            idt: sys::KvmDtable {
                base: 0x6000,
                limit: 0x40f,
                padding: [0; 3],
            },
            cr0: 1,
            cr2: 2,
            cr3: 3,
            cr4: 4,
            cr8: 5,
            efer: 6,
            apic_base: 7,
            interrupt_bitmap: [8, 9, 10, 11],
        });
        let host = HostMsrIndexList::from_validated_raw(&[0x10, 0x1b]);
        let indices = [MsrIndex::new(0x10), MsrIndex::new(0x1b)];
        let policy = GuestMsrAccessPolicy::from_host(&host, &indices).unwrap();
        let values = GuestMsrValueSet::from_policy(
            &policy,
            &[(indices[0], 0x1111), (indices[1], 0x2222)],
        )
        .unwrap();
        let msrs = GuestMsrSnapshot::from_capture(&policy, &values).unwrap();
        let vcpu = VcpuStateSnapshot {
            registers,
            special_registers,
            msrs,
        };
        let pages = vec![
            BoundedCheckpointPage {
                address: GuestPhysAddr::new(0x30000),
                bytes: vec![0x3a; LONG_MODE_PAGE_SIZE as usize],
            },
            BoundedCheckpointPage {
                address: GuestPhysAddr::new(0x31000),
                bytes: vec![0x4b; LONG_MODE_PAGE_SIZE as usize],
            },
        ];
        (BoundedVcpuPageSetCheckpoint { pages, vcpu }, host)
    }

    #[test]
    fn canonical_round_trip_reconstructs_the_checkpoint_semantically() {
        let (checkpoint, host) = checkpoint();
        let schema = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint).unwrap();
        let encoded = schema.encode().unwrap();
        let decoded = VersionedPageVcpuCheckpointV1::decode(&encoded).unwrap();
        let materialized = decoded.materialize(&host).unwrap();
        assert_eq!(schema, decoded);
        assert_eq!(checkpoint, materialized);
        assert_eq!(encoded.len(), encoded_length(2, 2).unwrap());
    }

    #[test]
    fn magic_version_architecture_and_lengths_are_fail_closed() {
        let (checkpoint, _) = checkpoint();
        let bytes = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint)
            .unwrap()
            .encode()
            .unwrap();

        let mut bad_magic = bytes.clone();
        bad_magic[0] ^= 0xff;
        assert_eq!(
            VersionedPageVcpuCheckpointV1::decode(&bad_magic),
            Err(VersionedPageVcpuCheckpointError::InvalidMagic)
        );

        let mut bad_version = bytes.clone();
        bad_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedPageVcpuCheckpointV1::decode(&bad_version),
            Err(VersionedPageVcpuCheckpointError::UnsupportedVersion(2))
        );

        let mut bad_arch = bytes.clone();
        bad_arch[10..12].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            VersionedPageVcpuCheckpointV1::decode(&bad_arch),
            Err(VersionedPageVcpuCheckpointError::UnsupportedArchitecture(2))
        );

        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(matches!(
            VersionedPageVcpuCheckpointV1::decode(&trailing),
            Err(VersionedPageVcpuCheckpointError::InvalidTotalLength { .. })
        ));
    }

    #[test]
    fn page_order_alignment_and_reserved_fields_are_validated() {
        let (checkpoint, _) = checkpoint();
        let bytes = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint)
            .unwrap()
            .encode()
            .unwrap();

        let mut unaligned = bytes.clone();
        unaligned[HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&0x30001_u64.to_le_bytes());
        assert_eq!(
            VersionedPageVcpuCheckpointV1::decode(&unaligned),
            Err(VersionedPageVcpuCheckpointError::UnalignedPage(0x30001))
        );

        let mut duplicate = bytes.clone();
        let second = HEADER_LEN + PAGE_RECORD_LEN;
        duplicate[second..second + 8].copy_from_slice(&0x30000_u64.to_le_bytes());
        assert!(matches!(
            VersionedPageVcpuCheckpointV1::decode(&duplicate),
            Err(VersionedPageVcpuCheckpointError::NonCanonicalPageOrder { .. })
        ));

        let mut flags = bytes.clone();
        flags[32..36].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            VersionedPageVcpuCheckpointV1::decode(&flags),
            Err(VersionedPageVcpuCheckpointError::NonZeroFlags(1))
        );
    }

    #[test]
    fn invalid_segment_bits_and_msr_order_are_rejected_before_materialization() {
        let (checkpoint, _) = checkpoint();
        let bytes = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint)
            .unwrap()
            .encode()
            .unwrap();
        let special_offset = HEADER_LEN + 2 * PAGE_RECORD_LEN + REGISTER_BLOCK_LEN;
        let mut invalid_present = bytes.clone();
        invalid_present[special_offset + 15] = 2;
        assert!(matches!(
            VersionedPageVcpuCheckpointV1::decode(&invalid_present),
            Err(VersionedPageVcpuCheckpointError::InvalidSegmentField {
                field: "present",
                ..
            })
        ));

        let msr_offset = special_offset + SPECIAL_REGISTER_BLOCK_LEN;
        let mut duplicate_msr = bytes;
        duplicate_msr[msr_offset + MSR_RECORD_LEN..msr_offset + MSR_RECORD_LEN + 4]
            .copy_from_slice(&0x10_u32.to_le_bytes());
        assert!(matches!(
            VersionedPageVcpuCheckpointV1::decode(&duplicate_msr),
            Err(VersionedPageVcpuCheckpointError::NonCanonicalMsrOrder { .. })
        ));
    }

    #[test]
    fn current_host_msr_support_is_revalidated_before_checkpoint_materialization() {
        let (checkpoint, _) = checkpoint();
        let schema = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint).unwrap();
        let incompatible_host = HostMsrIndexList::from_validated_raw(&[0x10]);
        assert!(matches!(
            schema.materialize(&incompatible_host),
            Err(VersionedPageVcpuCheckpointError::HostMsrIncompatible(_))
        ));
    }

    #[test]
    fn truncated_input_never_partially_decodes() {
        let (checkpoint, _) = checkpoint();
        let bytes = VersionedPageVcpuCheckpointV1::from_checkpoint(&checkpoint)
            .unwrap()
            .encode()
            .unwrap();
        for length in [0, 7, HEADER_LEN - 1] {
            assert!(VersionedPageVcpuCheckpointV1::decode(&bytes[..length]).is_err());
        }
    }
}
