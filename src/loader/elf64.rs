use crate::config::VmConfig;
use crate::error::Error;
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::KvmBackend;
use crate::long_mode::{
    LongModeBootLayout, LongModePageMapping, LONG_MODE_ALIAS_VIRTUAL_BASE,
    LONG_MODE_ALIAS_VIRTUAL_END, LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE,
    LONG_MODE_PAGE_TABLE_END, LONG_MODE_PML4_ADDR,
};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::PortIoBus;
use crate::vcpu::{PortIoExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::fmt;

const ELF64_HEADER_SIZE: usize = 64;
const ELF64_PROGRAM_HEADER_SIZE: usize = 56;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;

const PROOF_SEGMENT_PHYSICAL_ADDRESS: u64 = 0x1_0000;
const PROOF_SEGMENT_VIRTUAL_ADDRESS: u64 = LONG_MODE_ALIAS_VIRTUAL_BASE;
const PROOF_CODE_OFFSET: usize = 0x100;
const PROOF_ENTRY: u64 = PROOF_SEGMENT_VIRTUAL_ADDRESS + PROOF_CODE_OFFSET as u64;
const PROOF_MEMORY_SIZE: u64 = 0x180;
const PROOF_STACK_POINTER: u64 = 0x1f_f000;
const PROOF_EXIT_BUDGET: u32 = 5;
const PROOF_BYTES: &[u8; 4] = b"LM64";
const PROOF_CODE: [u8; 36] = [
    0x48, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x4c, 0x4d, 0x36, 0x34,
    0x48, 0xc1, 0xe8, 0x20,
    0xba, 0xe9, 0x00, 0x00, 0x00,
    0xee,
    0x48, 0xc1, 0xe8, 0x08, 0xee,
    0x48, 0xc1, 0xe8, 0x08, 0xee,
    0x48, 0xc1, 0xe8, 0x08, 0xee,
    0xf4,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Elf64Error {
    FileTooSmall {
        length: usize,
    },
    InvalidMagic,
    UnsupportedClass {
        actual: u8,
    },
    UnsupportedEndian {
        actual: u8,
    },
    UnsupportedIdentVersion {
        actual: u8,
    },
    UnsupportedType {
        actual: u16,
    },
    UnsupportedMachine {
        actual: u16,
    },
    UnsupportedVersion {
        actual: u32,
    },
    InvalidHeaderSize {
        actual: u16,
    },
    InvalidProgramHeaderSize {
        actual: u16,
    },
    ProgramHeaderTableOutOfBounds {
        offset: u64,
        count: u16,
        entry_size: u16,
        file_length: usize,
    },
    NoLoadableSegments,
    EmptyLoadSegment {
        index: u16,
    },
    SegmentFileSizeExceedsMemorySize {
        index: u16,
        file_size: u64,
        memory_size: u64,
    },
    SegmentFileRangeOutOfBounds {
        index: u16,
        offset: u64,
        file_size: u64,
        file_length: usize,
    },
    SegmentPhysicalRangeOverflow {
        index: u16,
        address: u64,
        memory_size: u64,
    },
    SegmentPhysicalRangeOutsideRam {
        index: u16,
        address: u64,
        memory_size: u64,
        ram_size: u64,
    },
    SegmentVirtualRangeOverflow {
        index: u16,
        address: u64,
        memory_size: u64,
    },
    SegmentVirtualRangeUnsupported {
        index: u16,
        address: u64,
        memory_size: u64,
    },
    IdentitySegmentAddressMismatch {
        index: u16,
        virtual_address: u64,
        physical_address: u64,
    },
    AliasPageOffsetMismatch {
        index: u16,
        virtual_address: u64,
        physical_address: u64,
    },
    SegmentOverlapsBootstrapPageTables {
        index: u16,
        address: u64,
        memory_size: u64,
    },
    InvalidSegmentAlignment {
        index: u16,
        alignment: u64,
    },
    SegmentAlignmentMismatch {
        index: u16,
        offset: u64,
        virtual_address: u64,
        alignment: u64,
    },
    LoadSegmentsVirtualOverlap {
        first: u16,
        second: u16,
    },
    LoadSegmentsPhysicalOverlap {
        first: u16,
        second: u16,
    },
    ConflictingPageMapping {
        virtual_page: u64,
        first_physical_page: u64,
        second_physical_page: u64,
    },
    EntryNotInExecutableFileBackedSegment {
        entry: u64,
    },
}

impl fmt::Display for Elf64Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooSmall { length } => {
                write!(f, "ELF64 image is too small for its fixed header: {length} bytes")
            }
            Self::InvalidMagic => write!(f, "ELF64 image has an invalid ELF magic"),
            Self::UnsupportedClass { actual } => {
                write!(f, "unsupported ELF class {actual}; expected ELFCLASS64")
            }
            Self::UnsupportedEndian { actual } => {
                write!(f, "unsupported ELF data encoding {actual}; expected little-endian")
            }
            Self::UnsupportedIdentVersion { actual } => {
                write!(f, "unsupported ELF identification version {actual}")
            }
            Self::UnsupportedType { actual } => {
                write!(f, "unsupported ELF type {actual}; this milestone accepts ET_EXEC only")
            }
            Self::UnsupportedMachine { actual } => {
                write!(f, "unsupported ELF machine {actual}; expected x86-64")
            }
            Self::UnsupportedVersion { actual } => write!(f, "unsupported ELF version {actual}"),
            Self::InvalidHeaderSize { actual } => {
                write!(f, "invalid ELF64 header size {actual}; expected {ELF64_HEADER_SIZE}")
            }
            Self::InvalidProgramHeaderSize { actual } => write!(
                f,
                "invalid ELF64 program-header size {actual}; expected {ELF64_PROGRAM_HEADER_SIZE}"
            ),
            Self::ProgramHeaderTableOutOfBounds {
                offset,
                count,
                entry_size,
                file_length,
            } => write!(
                f,
                "ELF64 program-header table is outside the file: offset={offset:#x}, count={count}, entry_size={entry_size}, file_length={file_length}"
            ),
            Self::NoLoadableSegments => write!(f, "ELF64 image has no PT_LOAD segment"),
            Self::EmptyLoadSegment { index } => {
                write!(f, "ELF64 PT_LOAD segment {index} has zero memory size")
            }
            Self::SegmentFileSizeExceedsMemorySize {
                index,
                file_size,
                memory_size,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} has filesz {file_size:#x} greater than memsz {memory_size:#x}"
            ),
            Self::SegmentFileRangeOutOfBounds {
                index,
                offset,
                file_size,
                file_length,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} file range is outside the image: offset={offset:#x}, filesz={file_size:#x}, file_length={file_length}"
            ),
            Self::SegmentPhysicalRangeOverflow {
                index,
                address,
                memory_size,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} physical range overflows: paddr={address:#x}, memsz={memory_size:#x}"
            ),
            Self::SegmentPhysicalRangeOutsideRam {
                index,
                address,
                memory_size,
                ram_size,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} physical backing is outside RAM: paddr={address:#x}, memsz={memory_size:#x}, ram_size={ram_size:#x}"
            ),
            Self::SegmentVirtualRangeOverflow {
                index,
                address,
                memory_size,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} virtual range overflows: vaddr={address:#x}, memsz={memory_size:#x}"
            ),
            Self::SegmentVirtualRangeUnsupported {
                index,
                address,
                memory_size,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} virtual range is outside the bounded identity/alias windows: vaddr={address:#x}, memsz={memory_size:#x}"
            ),
            Self::IdentitySegmentAddressMismatch {
                index,
                virtual_address,
                physical_address,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} lies in the identity window but vaddr={virtual_address:#x} differs from paddr={physical_address:#x}"
            ),
            Self::AliasPageOffsetMismatch {
                index,
                virtual_address,
                physical_address,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} alias mapping has different virtual/physical 4 KiB page offsets: vaddr={virtual_address:#x}, paddr={physical_address:#x}"
            ),
            Self::SegmentOverlapsBootstrapPageTables {
                index,
                address,
                memory_size,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} physical backing overlaps bootstrap page tables: paddr={address:#x}, memsz={memory_size:#x}"
            ),
            Self::InvalidSegmentAlignment { index, alignment } => write!(
                f,
                "ELF64 PT_LOAD segment {index} has invalid alignment {alignment:#x}; expected 0, 1, or a power of two"
            ),
            Self::SegmentAlignmentMismatch {
                index,
                offset,
                virtual_address,
                alignment,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} violates offset/vaddr alignment congruence: offset={offset:#x}, vaddr={virtual_address:#x}, align={alignment:#x}"
            ),
            Self::LoadSegmentsVirtualOverlap { first, second } => write!(
                f,
                "ELF64 PT_LOAD segments {first} and {second} overlap in virtual address space"
            ),
            Self::LoadSegmentsPhysicalOverlap { first, second } => write!(
                f,
                "ELF64 PT_LOAD segments {first} and {second} overlap in physical backing memory"
            ),
            Self::ConflictingPageMapping {
                virtual_page,
                first_physical_page,
                second_physical_page,
            } => write!(
                f,
                "ELF64 alias virtual page {virtual_page:#x} requires conflicting physical pages {first_physical_page:#x} and {second_physical_page:#x}"
            ),
            Self::EntryNotInExecutableFileBackedSegment { entry } => write!(
                f,
                "ELF64 entry {entry:#x} is not inside an executable file-backed PT_LOAD range"
            ),
        }
    }
}

impl std::error::Error for Elf64Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elf64LoadSegment {
    program_header_index: u16,
    file_offset: usize,
    file_size: usize,
    memory_size: usize,
    virtual_address: u64,
    physical_address: GuestPhysAddr,
    flags: u32,
}

impl Elf64LoadSegment {
    #[must_use]
    pub const fn program_header_index(&self) -> u16 {
        self.program_header_index
    }

    #[must_use]
    pub const fn virtual_address(&self) -> u64 {
        self.virtual_address
    }

    #[must_use]
    pub const fn physical_address(&self) -> GuestPhysAddr {
        self.physical_address
    }

    #[must_use]
    pub const fn file_size(&self) -> usize {
        self.file_size
    }

    #[must_use]
    pub const fn memory_size(&self) -> usize {
        self.memory_size
    }

    #[must_use]
    pub const fn executable(&self) -> bool {
        self.flags & PF_X != 0
    }

    #[must_use]
    pub const fn uses_alias_window(&self) -> bool {
        self.virtual_address >= LONG_MODE_ALIAS_VIRTUAL_BASE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elf64GuestImage<'a> {
    bytes: &'a [u8],
    entry: u64,
    segments: Vec<Elf64LoadSegment>,
    page_mappings: Vec<LongModePageMapping>,
}

impl<'a> Elf64GuestImage<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Elf64Error> {
        validate_ident_and_header(bytes)?;

        let entry = read_u64(bytes, 24);
        let program_header_offset = read_u64(bytes, 32);
        let program_header_size = read_u16(bytes, 54);
        let program_header_count = read_u16(bytes, 56);
        let table_offset = usize::try_from(program_header_offset).map_err(|_| {
            program_header_bounds_error(
                program_header_offset,
                program_header_count,
                program_header_size,
                bytes.len(),
            )
        })?;
        let table_size = usize::from(program_header_count)
            .checked_mul(ELF64_PROGRAM_HEADER_SIZE)
            .ok_or_else(|| {
                program_header_bounds_error(
                    program_header_offset,
                    program_header_count,
                    program_header_size,
                    bytes.len(),
                )
            })?;
        let table_end = table_offset.checked_add(table_size).ok_or_else(|| {
            program_header_bounds_error(
                program_header_offset,
                program_header_count,
                program_header_size,
                bytes.len(),
            )
        })?;
        if table_end > bytes.len() {
            return Err(program_header_bounds_error(
                program_header_offset,
                program_header_count,
                program_header_size,
                bytes.len(),
            ));
        }

        let mut segments = Vec::new();
        for index in 0..program_header_count {
            let offset = table_offset + usize::from(index) * ELF64_PROGRAM_HEADER_SIZE;
            if read_u32(bytes, offset) != PT_LOAD {
                continue;
            }
            let segment = parse_load_segment(bytes, offset, index)?;
            reject_segment_overlap(&segments, &segment)?;
            segments.push(segment);
        }

        if segments.is_empty() {
            return Err(Elf64Error::NoLoadableSegments);
        }
        if !segments.iter().any(|segment| {
            let file_end = segment.virtual_address + segment.file_size as u64;
            segment.executable() && entry >= segment.virtual_address && entry < file_end
        }) {
            return Err(Elf64Error::EntryNotInExecutableFileBackedSegment { entry });
        }

        let page_mappings = build_page_mappings(&segments)?;
        Ok(Self {
            bytes,
            entry,
            segments,
            page_mappings,
        })
    }

    #[must_use]
    pub const fn entry(&self) -> u64 {
        self.entry
    }

    #[must_use]
    pub fn segments(&self) -> &[Elf64LoadSegment] {
        &self.segments
    }

    #[must_use]
    pub fn page_mappings(&self) -> &[LongModePageMapping] {
        &self.page_mappings
    }

    pub fn load(&self, memory: &mut GuestMemory) -> Result<(), Error> {
        for segment in &self.segments {
            let file_end = segment.file_offset + segment.file_size;
            memory.write(
                segment.physical_address,
                &self.bytes[segment.file_offset..file_end],
            )?;

            if segment.memory_size > segment.file_size {
                let zero_length = segment.memory_size - segment.file_size;
                let zero_address = GuestPhysAddr::new(
                    segment.physical_address.get() + segment.file_size as u64,
                );
                memory.write(zero_address, &vec![0_u8; zero_length])?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elf64GuestResult {
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
}

impl Elf64GuestResult {
    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn report(&self) -> VmExitReport {
        self.report
    }
}

pub fn run_elf64_guest(config: VmConfig) -> Result<Elf64GuestResult, Error> {
    let bytes = proof_fixture();
    let image = Elf64GuestImage::parse(&bytes)
        .expect("the built-in non-identity ELF64 proof fixture remains structurally valid");
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeBootLayout::with_page_mappings(
        memory.region(),
        image.entry(),
        PROOF_STACK_POINTER,
        image.page_mappings().to_vec(),
    )
    .expect("the validated ELF64 proof mappings remain inside the bounded long-mode layout");
    layout.install_page_tables(&mut memory)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode(&layout)?;
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(&mut vcpu, &mut port_io, PROOF_EXIT_BUDGET)?;
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();

    Ok(Elf64GuestResult {
        io_exits: execution.io_exits().to_vec(),
        proof,
        report: execution.report(),
    })
}

#[must_use]
pub const fn proof_terminal_rip() -> u64 {
    PROOF_ENTRY + PROOF_CODE.len() as u64
}

#[must_use]
pub const fn expected_proof() -> &'static [u8] {
    PROOF_BYTES
}

fn validate_ident_and_header(bytes: &[u8]) -> Result<(), Elf64Error> {
    if bytes.len() < ELF64_HEADER_SIZE {
        return Err(Elf64Error::FileTooSmall {
            length: bytes.len(),
        });
    }
    if bytes[0..4] != [0x7f, b'E', b'L', b'F'] {
        return Err(Elf64Error::InvalidMagic);
    }
    if bytes[4] != ELFCLASS64 {
        return Err(Elf64Error::UnsupportedClass { actual: bytes[4] });
    }
    if bytes[5] != ELFDATA2LSB {
        return Err(Elf64Error::UnsupportedEndian { actual: bytes[5] });
    }
    if bytes[6] != EV_CURRENT {
        return Err(Elf64Error::UnsupportedIdentVersion { actual: bytes[6] });
    }
    let elf_type = read_u16(bytes, 16);
    if elf_type != ET_EXEC {
        return Err(Elf64Error::UnsupportedType { actual: elf_type });
    }
    let machine = read_u16(bytes, 18);
    if machine != EM_X86_64 {
        return Err(Elf64Error::UnsupportedMachine { actual: machine });
    }
    let version = read_u32(bytes, 20);
    if version != u32::from(EV_CURRENT) {
        return Err(Elf64Error::UnsupportedVersion { actual: version });
    }
    let header_size = read_u16(bytes, 52);
    if usize::from(header_size) != ELF64_HEADER_SIZE {
        return Err(Elf64Error::InvalidHeaderSize {
            actual: header_size,
        });
    }
    let program_header_size = read_u16(bytes, 54);
    if usize::from(program_header_size) != ELF64_PROGRAM_HEADER_SIZE {
        return Err(Elf64Error::InvalidProgramHeaderSize {
            actual: program_header_size,
        });
    }
    Ok(())
}

fn program_header_bounds_error(
    offset: u64,
    count: u16,
    entry_size: u16,
    file_length: usize,
) -> Elf64Error {
    Elf64Error::ProgramHeaderTableOutOfBounds {
        offset,
        count,
        entry_size,
        file_length,
    }
}

fn parse_load_segment(
    bytes: &[u8],
    offset: usize,
    index: u16,
) -> Result<Elf64LoadSegment, Elf64Error> {
    let flags = read_u32(bytes, offset + 4);
    let file_offset_u64 = read_u64(bytes, offset + 8);
    let virtual_address = read_u64(bytes, offset + 16);
    let physical_address = read_u64(bytes, offset + 24);
    let file_size_u64 = read_u64(bytes, offset + 32);
    let memory_size_u64 = read_u64(bytes, offset + 40);
    let alignment = read_u64(bytes, offset + 48);

    if memory_size_u64 == 0 {
        return Err(Elf64Error::EmptyLoadSegment { index });
    }
    if file_size_u64 > memory_size_u64 {
        return Err(Elf64Error::SegmentFileSizeExceedsMemorySize {
            index,
            file_size: file_size_u64,
            memory_size: memory_size_u64,
        });
    }
    if alignment > 1 && !alignment.is_power_of_two() {
        return Err(Elf64Error::InvalidSegmentAlignment { index, alignment });
    }
    if alignment > 1 && file_offset_u64 % alignment != virtual_address % alignment {
        return Err(Elf64Error::SegmentAlignmentMismatch {
            index,
            offset: file_offset_u64,
            virtual_address,
            alignment,
        });
    }

    let file_offset = usize::try_from(file_offset_u64).map_err(|_| {
        Elf64Error::SegmentFileRangeOutOfBounds {
            index,
            offset: file_offset_u64,
            file_size: file_size_u64,
            file_length: bytes.len(),
        }
    })?;
    let file_size = usize::try_from(file_size_u64).map_err(|_| {
        Elf64Error::SegmentFileRangeOutOfBounds {
            index,
            offset: file_offset_u64,
            file_size: file_size_u64,
            file_length: bytes.len(),
        }
    })?;
    let file_end = file_offset.checked_add(file_size).ok_or(
        Elf64Error::SegmentFileRangeOutOfBounds {
            index,
            offset: file_offset_u64,
            file_size: file_size_u64,
            file_length: bytes.len(),
        },
    )?;
    if file_end > bytes.len() {
        return Err(Elf64Error::SegmentFileRangeOutOfBounds {
            index,
            offset: file_offset_u64,
            file_size: file_size_u64,
            file_length: bytes.len(),
        });
    }

    let physical_end = physical_address.checked_add(memory_size_u64).ok_or(
        Elf64Error::SegmentPhysicalRangeOverflow {
            index,
            address: physical_address,
            memory_size: memory_size_u64,
        },
    )?;
    if physical_end > LONG_MODE_IDENTITY_MAP_SIZE {
        return Err(Elf64Error::SegmentPhysicalRangeOutsideRam {
            index,
            address: physical_address,
            memory_size: memory_size_u64,
            ram_size: LONG_MODE_IDENTITY_MAP_SIZE,
        });
    }
    if ranges_overlap(
        physical_address,
        physical_end,
        LONG_MODE_PML4_ADDR.get(),
        LONG_MODE_PAGE_TABLE_END.get(),
    ) {
        return Err(Elf64Error::SegmentOverlapsBootstrapPageTables {
            index,
            address: physical_address,
            memory_size: memory_size_u64,
        });
    }

    let virtual_end = virtual_address.checked_add(memory_size_u64).ok_or(
        Elf64Error::SegmentVirtualRangeOverflow {
            index,
            address: virtual_address,
            memory_size: memory_size_u64,
        },
    )?;
    let uses_identity_window = virtual_end <= LONG_MODE_IDENTITY_MAP_SIZE;
    let uses_alias_window = virtual_address >= LONG_MODE_ALIAS_VIRTUAL_BASE
        && virtual_end <= LONG_MODE_ALIAS_VIRTUAL_END;
    if !uses_identity_window && !uses_alias_window {
        return Err(Elf64Error::SegmentVirtualRangeUnsupported {
            index,
            address: virtual_address,
            memory_size: memory_size_u64,
        });
    }
    if uses_identity_window && virtual_address != physical_address {
        return Err(Elf64Error::IdentitySegmentAddressMismatch {
            index,
            virtual_address,
            physical_address,
        });
    }
    if uses_alias_window
        && virtual_address % LONG_MODE_PAGE_SIZE != physical_address % LONG_MODE_PAGE_SIZE
    {
        return Err(Elf64Error::AliasPageOffsetMismatch {
            index,
            virtual_address,
            physical_address,
        });
    }

    let memory_size = usize::try_from(memory_size_u64).map_err(|_| {
        Elf64Error::SegmentPhysicalRangeOutsideRam {
            index,
            address: physical_address,
            memory_size: memory_size_u64,
            ram_size: LONG_MODE_IDENTITY_MAP_SIZE,
        }
    })?;
    Ok(Elf64LoadSegment {
        program_header_index: index,
        file_offset,
        file_size,
        memory_size,
        virtual_address,
        physical_address: GuestPhysAddr::new(physical_address),
        flags,
    })
}

fn reject_segment_overlap(
    existing: &[Elf64LoadSegment],
    candidate: &Elf64LoadSegment,
) -> Result<(), Elf64Error> {
    let candidate_virtual_end = candidate.virtual_address + candidate.memory_size as u64;
    let candidate_physical_end = candidate.physical_address.get() + candidate.memory_size as u64;
    for segment in existing {
        let virtual_end = segment.virtual_address + segment.memory_size as u64;
        if ranges_overlap(
            candidate.virtual_address,
            candidate_virtual_end,
            segment.virtual_address,
            virtual_end,
        ) {
            return Err(Elf64Error::LoadSegmentsVirtualOverlap {
                first: segment.program_header_index,
                second: candidate.program_header_index,
            });
        }
        let physical_end = segment.physical_address.get() + segment.memory_size as u64;
        if ranges_overlap(
            candidate.physical_address.get(),
            candidate_physical_end,
            segment.physical_address.get(),
            physical_end,
        ) {
            return Err(Elf64Error::LoadSegmentsPhysicalOverlap {
                first: segment.program_header_index,
                second: candidate.program_header_index,
            });
        }
    }
    Ok(())
}

fn build_page_mappings(
    segments: &[Elf64LoadSegment],
) -> Result<Vec<LongModePageMapping>, Elf64Error> {
    let mut mappings: Vec<LongModePageMapping> = Vec::new();
    for segment in segments.iter().filter(|segment| segment.uses_alias_window()) {
        let virtual_start = align_down_page(segment.virtual_address);
        let physical_start = align_down_page(segment.physical_address.get());
        let virtual_end = align_up_page(segment.virtual_address + segment.memory_size as u64);
        let page_count = (virtual_end - virtual_start) / LONG_MODE_PAGE_SIZE;
        for page in 0..page_count {
            let virtual_page = virtual_start + page * LONG_MODE_PAGE_SIZE;
            let physical_page = physical_start + page * LONG_MODE_PAGE_SIZE;
            if let Some(existing) = mappings
                .iter()
                .find(|mapping| mapping.virtual_page() == virtual_page)
            {
                if existing.physical_page().get() != physical_page {
                    return Err(Elf64Error::ConflictingPageMapping {
                        virtual_page,
                        first_physical_page: existing.physical_page().get(),
                        second_physical_page: physical_page,
                    });
                }
                continue;
            }
            mappings.push(LongModePageMapping::new(
                virtual_page,
                GuestPhysAddr::new(physical_page),
            ));
        }
    }
    Ok(mappings)
}

fn proof_fixture() -> Vec<u8> {
    fixture_with_addresses(
        PROOF_SEGMENT_VIRTUAL_ADDRESS,
        PROOF_SEGMENT_PHYSICAL_ADDRESS,
        &PROOF_CODE,
        PROOF_MEMORY_SIZE,
    )
}

fn fixture_with_addresses(
    virtual_address: u64,
    physical_address: u64,
    code: &[u8],
    memory_size: u64,
) -> Vec<u8> {
    let file_size = PROOF_CODE_OFFSET + code.len();
    let mut bytes = vec![0_u8; file_size];
    bytes[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    bytes[4] = ELFCLASS64;
    bytes[5] = ELFDATA2LSB;
    bytes[6] = EV_CURRENT;
    write_u16(&mut bytes, 16, ET_EXEC);
    write_u16(&mut bytes, 18, EM_X86_64);
    write_u32(&mut bytes, 20, u32::from(EV_CURRENT));
    write_u64(&mut bytes, 24, virtual_address + PROOF_CODE_OFFSET as u64);
    write_u64(&mut bytes, 32, ELF64_HEADER_SIZE as u64);
    write_u16(&mut bytes, 52, ELF64_HEADER_SIZE as u16);
    write_u16(&mut bytes, 54, ELF64_PROGRAM_HEADER_SIZE as u16);
    write_u16(&mut bytes, 56, 1);

    let ph = ELF64_HEADER_SIZE;
    write_u32(&mut bytes, ph, PT_LOAD);
    write_u32(&mut bytes, ph + 4, PF_X | 4);
    write_u64(&mut bytes, ph + 8, 0);
    write_u64(&mut bytes, ph + 16, virtual_address);
    write_u64(&mut bytes, ph + 24, physical_address);
    write_u64(&mut bytes, ph + 32, file_size as u64);
    write_u64(&mut bytes, ph + 40, memory_size);
    write_u64(&mut bytes, ph + 48, LONG_MODE_PAGE_SIZE);
    bytes[PROOF_CODE_OFFSET..file_size].copy_from_slice(code);
    bytes
}

const fn align_down_page(address: u64) -> u64 {
    address & !(LONG_MODE_PAGE_SIZE - 1)
}

const fn align_up_page(address: u64) -> u64 {
    (address + LONG_MODE_PAGE_SIZE - 1) & !(LONG_MODE_PAGE_SIZE - 1)
}

const fn ranges_overlap(
    first_start: u64,
    first_end: u64,
    second_start: u64,
    second_end: u64,
) -> bool {
    first_start < second_end && second_start < first_end
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CODE: [u8; 2] = [0x90, 0xf4];

    fn alias_fixture() -> Vec<u8> {
        fixture_with_addresses(
            PROOF_SEGMENT_VIRTUAL_ADDRESS,
            PROOF_SEGMENT_PHYSICAL_ADDRESS,
            &TEST_CODE,
            PROOF_MEMORY_SIZE,
        )
    }

    fn identity_fixture() -> Vec<u8> {
        fixture_with_addresses(
            PROOF_SEGMENT_PHYSICAL_ADDRESS,
            PROOF_SEGMENT_PHYSICAL_ADDRESS,
            &TEST_CODE,
            PROOF_MEMORY_SIZE,
        )
    }

    #[test]
    fn parses_nonidentity_alias_segment_and_builds_page_mapping() {
        let bytes = alias_fixture();
        let image = Elf64GuestImage::parse(&bytes).unwrap();
        let segment = image.segments()[0];

        assert_eq!(image.entry(), PROOF_ENTRY);
        assert_eq!(segment.virtual_address(), PROOF_SEGMENT_VIRTUAL_ADDRESS);
        assert_eq!(
            segment.physical_address().get(),
            PROOF_SEGMENT_PHYSICAL_ADDRESS
        );
        assert!(segment.uses_alias_window());
        assert_eq!(
            image.page_mappings(),
            &[LongModePageMapping::new(
                PROOF_SEGMENT_VIRTUAL_ADDRESS,
                GuestPhysAddr::new(PROOF_SEGMENT_PHYSICAL_ADDRESS),
            )]
        );
    }

    #[test]
    fn preserves_identity_mapped_elf_acceptance_without_alias_pte() {
        let bytes = identity_fixture();
        let image = Elf64GuestImage::parse(&bytes).unwrap();
        let segment = image.segments()[0];

        assert_eq!(image.entry(), PROOF_SEGMENT_PHYSICAL_ADDRESS + 0x100);
        assert_eq!(segment.virtual_address(), segment.physical_address().get());
        assert!(!segment.uses_alias_window());
        assert!(image.page_mappings().is_empty());
    }

    #[test]
    fn load_uses_physical_backing_and_explicitly_zeroes_bss() {
        let bytes = alias_fixture();
        let image = Elf64GuestImage::parse(&bytes).unwrap();
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let dirty = vec![0xaa; PROOF_MEMORY_SIZE as usize];
        memory
            .write(
                GuestPhysAddr::new(PROOF_SEGMENT_PHYSICAL_ADDRESS),
                &dirty,
            )
            .unwrap();

        image.load(&mut memory).unwrap();

        let mut observed_file = vec![0_u8; bytes.len()];
        memory
            .read(
                GuestPhysAddr::new(PROOF_SEGMENT_PHYSICAL_ADDRESS),
                &mut observed_file,
            )
            .unwrap();
        assert_eq!(observed_file, bytes);
        let mut observed_bss = vec![0xff; PROOF_MEMORY_SIZE as usize - bytes.len()];
        memory
            .read(
                GuestPhysAddr::new(PROOF_SEGMENT_PHYSICAL_ADDRESS + bytes.len() as u64),
                &mut observed_bss,
            )
            .unwrap();
        assert!(observed_bss.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn rejects_non_elf_and_non_x86_64_inputs() {
        let mut bytes = alias_fixture();
        bytes[0] = 0;
        assert_eq!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::InvalidMagic)
        );

        let mut bytes = alias_fixture();
        write_u16(&mut bytes, 18, 3);
        assert_eq!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::UnsupportedMachine { actual: 3 })
        );
    }

    #[test]
    fn rejects_filesz_larger_than_memsz_and_file_range_overrun() {
        let ph = ELF64_HEADER_SIZE;
        let mut bytes = alias_fixture();
        write_u64(&mut bytes, ph + 40, 1);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentFileSizeExceedsMemorySize { .. })
        ));

        let mut bytes = alias_fixture();
        let too_large = bytes.len() as u64 + 1;
        write_u64(&mut bytes, ph + 32, too_large);
        write_u64(&mut bytes, ph + 40, too_large);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentFileRangeOutOfBounds { .. })
        ));
    }

    #[test]
    fn rejects_physical_backing_outside_ram_or_overlapping_page_tables() {
        let ph = ELF64_HEADER_SIZE;
        let mut bytes = alias_fixture();
        write_u64(&mut bytes, ph + 24, LONG_MODE_IDENTITY_MAP_SIZE);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentPhysicalRangeOutsideRam { .. })
        ));

        let mut bytes = alias_fixture();
        write_u64(&mut bytes, ph + 24, LONG_MODE_PML4_ADDR.get());
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentOverlapsBootstrapPageTables { .. })
        ));
    }

    #[test]
    fn rejects_alias_page_offset_mismatch_and_virtual_range_outside_window() {
        let ph = ELF64_HEADER_SIZE;
        let mut bytes = alias_fixture();
        write_u64(&mut bytes, ph + 24, PROOF_SEGMENT_PHYSICAL_ADDRESS + 1);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::AliasPageOffsetMismatch { .. })
        ));

        let mut bytes = alias_fixture();
        write_u64(&mut bytes, ph + 16, LONG_MODE_IDENTITY_MAP_SIZE + LONG_MODE_PAGE_SIZE);
        write_u64(
            &mut bytes,
            24,
            LONG_MODE_IDENTITY_MAP_SIZE + LONG_MODE_PAGE_SIZE + PROOF_CODE_OFFSET as u64,
        );
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentVirtualRangeUnsupported { .. })
        ));
    }

    #[test]
    fn rejects_identity_segment_with_nonidentity_physical_address() {
        let ph = ELF64_HEADER_SIZE;
        let mut bytes = identity_fixture();
        write_u64(&mut bytes, ph + 24, PROOF_SEGMENT_PHYSICAL_ADDRESS + LONG_MODE_PAGE_SIZE);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::IdentitySegmentAddressMismatch { .. })
        ));
    }

    #[test]
    fn rejects_entry_outside_executable_file_backed_range() {
        let mut bytes = alias_fixture();
        let entry = PROOF_SEGMENT_VIRTUAL_ADDRESS + PROOF_MEMORY_SIZE - 1;
        write_u64(&mut bytes, 24, entry);
        assert_eq!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::EntryNotInExecutableFileBackedSegment { entry })
        );
    }

    #[test]
    fn rejects_invalid_or_incongruent_elf_segment_alignment() {
        let ph = ELF64_HEADER_SIZE;
        let mut bytes = alias_fixture();
        write_u64(&mut bytes, ph + 48, 3);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::InvalidSegmentAlignment { .. })
        ));

        let mut bytes = alias_fixture();
        write_u64(&mut bytes, ph + 8, 1);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentAlignmentMismatch { .. })
        ));
    }

    #[test]
    fn deterministic_fixture_uses_virtual_entry_and_terminal_rip() {
        let bytes = proof_fixture();
        let image = Elf64GuestImage::parse(&bytes).unwrap();
        assert_eq!(image.entry(), 0x40_0100);
        assert_eq!(proof_terminal_rip(), 0x40_0124);
        assert_eq!(expected_proof(), b"LM64");
    }
}
