use crate::error::Error;
use crate::long_mode::{
    LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_TABLE_END, LONG_MODE_PML4_ADDR,
};
use crate::memory::{GuestMemory, GuestPhysAddr};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Elf64Error {
    FileTooSmall { length: usize },
    InvalidMagic,
    UnsupportedClass { actual: u8 },
    UnsupportedEndian { actual: u8 },
    UnsupportedIdentVersion { actual: u8 },
    UnsupportedType { actual: u16 },
    UnsupportedMachine { actual: u16 },
    UnsupportedVersion { actual: u32 },
    InvalidHeaderSize { actual: u16 },
    InvalidProgramHeaderSize { actual: u16 },
    ProgramHeaderTableOutOfBounds {
        offset: u64,
        count: u16,
        entry_size: u16,
        file_length: usize,
    },
    NoLoadableSegments,
    EmptyLoadSegment { index: u16 },
    SegmentVirtualPhysicalMismatch {
        index: u16,
        virtual_address: u64,
        physical_address: u64,
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
    SegmentAddressRangeOverflow {
        index: u16,
        address: u64,
        memory_size: u64,
    },
    SegmentOutsideIdentityMap {
        index: u16,
        address: u64,
        memory_size: u64,
        mapped_size: u64,
    },
    SegmentOverlapsBootstrapPageTables {
        index: u16,
        address: u64,
        memory_size: u64,
    },
    InvalidSegmentAlignment { index: u16, alignment: u64 },
    SegmentAlignmentMismatch {
        index: u16,
        offset: u64,
        virtual_address: u64,
        alignment: u64,
    },
    LoadSegmentsOverlap { first: u16, second: u16 },
    EntryNotInExecutableFileBackedSegment { entry: u64 },
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
            Self::SegmentVirtualPhysicalMismatch {
                index,
                virtual_address,
                physical_address,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} requires virtual==physical under the current identity-map contract: vaddr={virtual_address:#x}, paddr={physical_address:#x}"
            ),
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
            Self::SegmentAddressRangeOverflow {
                index,
                address,
                memory_size,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} guest range overflows: address={address:#x}, memsz={memory_size:#x}"
            ),
            Self::SegmentOutsideIdentityMap {
                index,
                address,
                memory_size,
                mapped_size,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} is outside the current identity map: address={address:#x}, memsz={memory_size:#x}, mapped_size={mapped_size:#x}"
            ),
            Self::SegmentOverlapsBootstrapPageTables {
                index,
                address,
                memory_size,
            } => write!(
                f,
                "ELF64 PT_LOAD segment {index} overlaps reserved bootstrap page tables: address={address:#x}, memsz={memory_size:#x}"
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
            Self::LoadSegmentsOverlap { first, second } => write!(
                f,
                "ELF64 PT_LOAD segments {first} and {second} overlap in guest memory"
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
    guest_address: GuestPhysAddr,
    flags: u32,
}

impl Elf64LoadSegment {
    #[must_use]
    pub const fn program_header_index(&self) -> u16 {
        self.program_header_index
    }

    #[must_use]
    pub const fn guest_address(&self) -> GuestPhysAddr {
        self.guest_address
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elf64GuestImage<'a> {
    bytes: &'a [u8],
    entry: GuestPhysAddr,
    segments: Vec<Elf64LoadSegment>,
}

impl<'a> Elf64GuestImage<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Elf64Error> {
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

        let entry = read_u64(bytes, 24);
        let program_header_offset = read_u64(bytes, 32);
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
        let program_header_count = read_u16(bytes, 56);
        let table_offset = usize::try_from(program_header_offset).map_err(|_| {
            Elf64Error::ProgramHeaderTableOutOfBounds {
                offset: program_header_offset,
                count: program_header_count,
                entry_size: program_header_size,
                file_length: bytes.len(),
            }
        })?;
        let table_size = usize::from(program_header_count)
            .checked_mul(ELF64_PROGRAM_HEADER_SIZE)
            .ok_or(Elf64Error::ProgramHeaderTableOutOfBounds {
                offset: program_header_offset,
                count: program_header_count,
                entry_size: program_header_size,
                file_length: bytes.len(),
            })?;
        let table_end = table_offset
            .checked_add(table_size)
            .ok_or(Elf64Error::ProgramHeaderTableOutOfBounds {
                offset: program_header_offset,
                count: program_header_count,
                entry_size: program_header_size,
                file_length: bytes.len(),
            })?;
        if table_end > bytes.len() {
            return Err(Elf64Error::ProgramHeaderTableOutOfBounds {
                offset: program_header_offset,
                count: program_header_count,
                entry_size: program_header_size,
                file_length: bytes.len(),
            });
        }

        let mut segments = Vec::new();
        for index in 0..program_header_count {
            let offset = table_offset + usize::from(index) * ELF64_PROGRAM_HEADER_SIZE;
            if read_u32(bytes, offset) != PT_LOAD {
                continue;
            }

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
            if virtual_address != physical_address {
                return Err(Elf64Error::SegmentVirtualPhysicalMismatch {
                    index,
                    virtual_address,
                    physical_address,
                });
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

            let guest_end = virtual_address.checked_add(memory_size_u64).ok_or(
                Elf64Error::SegmentAddressRangeOverflow {
                    index,
                    address: virtual_address,
                    memory_size: memory_size_u64,
                },
            )?;
            if guest_end > LONG_MODE_IDENTITY_MAP_SIZE {
                return Err(Elf64Error::SegmentOutsideIdentityMap {
                    index,
                    address: virtual_address,
                    memory_size: memory_size_u64,
                    mapped_size: LONG_MODE_IDENTITY_MAP_SIZE,
                });
            }
            if ranges_overlap(
                virtual_address,
                guest_end,
                LONG_MODE_PML4_ADDR.get(),
                LONG_MODE_PAGE_TABLE_END.get(),
            ) {
                return Err(Elf64Error::SegmentOverlapsBootstrapPageTables {
                    index,
                    address: virtual_address,
                    memory_size: memory_size_u64,
                });
            }

            for existing in &segments {
                let existing_start = existing.guest_address.get();
                let existing_end = existing_start + existing.memory_size as u64;
                if ranges_overlap(virtual_address, guest_end, existing_start, existing_end) {
                    return Err(Elf64Error::LoadSegmentsOverlap {
                        first: existing.program_header_index,
                        second: index,
                    });
                }
            }

            let memory_size = usize::try_from(memory_size_u64).map_err(|_| {
                Elf64Error::SegmentOutsideIdentityMap {
                    index,
                    address: virtual_address,
                    memory_size: memory_size_u64,
                    mapped_size: LONG_MODE_IDENTITY_MAP_SIZE,
                }
            })?;
            segments.push(Elf64LoadSegment {
                program_header_index: index,
                file_offset,
                file_size,
                memory_size,
                guest_address: GuestPhysAddr::new(virtual_address),
                flags,
            });
        }

        if segments.is_empty() {
            return Err(Elf64Error::NoLoadableSegments);
        }
        let entry_is_executable = segments.iter().any(|segment| {
            if !segment.executable() {
                return false;
            }
            let start = segment.guest_address.get();
            let end = start + segment.file_size as u64;
            entry >= start && entry < end
        });
        if !entry_is_executable {
            return Err(Elf64Error::EntryNotInExecutableFileBackedSegment { entry });
        }

        Ok(Self {
            bytes,
            entry: GuestPhysAddr::new(entry),
            segments,
        })
    }

    #[must_use]
    pub const fn entry(&self) -> GuestPhysAddr {
        self.entry
    }

    #[must_use]
    pub fn segments(&self) -> &[Elf64LoadSegment] {
        &self.segments
    }

    pub fn load(&self, memory: &mut GuestMemory) -> Result<(), Error> {
        for segment in &self.segments {
            let file_end = segment.file_offset + segment.file_size;
            memory.write(
                segment.guest_address,
                &self.bytes[segment.file_offset..file_end],
            )?;

            if segment.memory_size > segment.file_size {
                let zero_length = segment.memory_size - segment.file_size;
                let zero_address = GuestPhysAddr::new(
                    segment.guest_address.get() + segment.file_size as u64,
                );
                let zeros = vec![0_u8; zero_length];
                memory.write(zero_address, &zeros)?;
            }
        }
        Ok(())
    }
}

const fn ranges_overlap(first_start: u64, first_end: u64, second_start: u64, second_end: u64) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    const SEGMENT_ADDRESS: u64 = 0x1_0000;
    const CODE_OFFSET: usize = 0x100;
    const CODE: [u8; 2] = [0x90, 0xf4];

    fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn fixture() -> Vec<u8> {
        let file_size = CODE_OFFSET + CODE.len();
        let mut bytes = vec![0_u8; file_size];
        bytes[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        bytes[4] = ELFCLASS64;
        bytes[5] = ELFDATA2LSB;
        bytes[6] = EV_CURRENT;
        write_u16(&mut bytes, 16, ET_EXEC);
        write_u16(&mut bytes, 18, EM_X86_64);
        write_u32(&mut bytes, 20, u32::from(EV_CURRENT));
        write_u64(&mut bytes, 24, SEGMENT_ADDRESS + CODE_OFFSET as u64);
        write_u64(&mut bytes, 32, ELF64_HEADER_SIZE as u64);
        write_u16(&mut bytes, 52, ELF64_HEADER_SIZE as u16);
        write_u16(&mut bytes, 54, ELF64_PROGRAM_HEADER_SIZE as u16);
        write_u16(&mut bytes, 56, 1);

        let ph = ELF64_HEADER_SIZE;
        write_u32(&mut bytes, ph, PT_LOAD);
        write_u32(&mut bytes, ph + 4, PF_X | 4);
        write_u64(&mut bytes, ph + 8, 0);
        write_u64(&mut bytes, ph + 16, SEGMENT_ADDRESS);
        write_u64(&mut bytes, ph + 24, SEGMENT_ADDRESS);
        write_u64(&mut bytes, ph + 32, file_size as u64);
        write_u64(&mut bytes, ph + 40, 0x180);
        write_u64(&mut bytes, ph + 48, 0x1000);
        bytes[CODE_OFFSET..CODE_OFFSET + CODE.len()].copy_from_slice(&CODE);
        bytes
    }

    #[test]
    fn parses_bounded_identity_mapped_x86_64_executable() {
        let bytes = fixture();
        let image = Elf64GuestImage::parse(&bytes).unwrap();

        assert_eq!(image.entry().get(), SEGMENT_ADDRESS + CODE_OFFSET as u64);
        assert_eq!(image.segments().len(), 1);
        let segment = image.segments()[0];
        assert_eq!(segment.program_header_index(), 0);
        assert_eq!(segment.guest_address().get(), SEGMENT_ADDRESS);
        assert_eq!(segment.file_size(), bytes.len());
        assert_eq!(segment.memory_size(), 0x180);
        assert!(segment.executable());
    }

    #[test]
    fn load_copies_file_bytes_and_explicitly_zeroes_bss() {
        let bytes = fixture();
        let image = Elf64GuestImage::parse(&bytes).unwrap();
        let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let dirty = vec![0xaa; 0x180];
        memory
            .write(GuestPhysAddr::new(SEGMENT_ADDRESS), &dirty)
            .unwrap();

        image.load(&mut memory).unwrap();

        let mut observed_file = vec![0_u8; bytes.len()];
        memory
            .read(GuestPhysAddr::new(SEGMENT_ADDRESS), &mut observed_file)
            .unwrap();
        assert_eq!(observed_file, bytes);
        let mut observed_bss = vec![0xff; 0x180 - bytes.len()];
        memory
            .read(
                GuestPhysAddr::new(SEGMENT_ADDRESS + bytes.len() as u64),
                &mut observed_bss,
            )
            .unwrap();
        assert!(observed_bss.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn rejects_non_elf_and_non_x86_64_inputs() {
        let mut bytes = fixture();
        bytes[0] = 0;
        assert_eq!(Elf64GuestImage::parse(&bytes), Err(Elf64Error::InvalidMagic));

        let mut bytes = fixture();
        write_u16(&mut bytes, 18, 3);
        assert_eq!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::UnsupportedMachine { actual: 3 })
        );
    }

    #[test]
    fn rejects_filesz_larger_than_memsz_and_file_range_overrun() {
        let mut bytes = fixture();
        let ph = ELF64_HEADER_SIZE;
        write_u64(&mut bytes, ph + 40, 1);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentFileSizeExceedsMemorySize { .. })
        ));

        let mut bytes = fixture();
        write_u64(&mut bytes, ph + 32, bytes.len() as u64 + 1);
        write_u64(&mut bytes, ph + 40, bytes.len() as u64 + 1);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentFileRangeOutOfBounds { .. })
        ));
    }

    #[test]
    fn rejects_non_identity_mapped_or_bootstrap_overlapping_segments() {
        let mut bytes = fixture();
        let ph = ELF64_HEADER_SIZE;
        write_u64(&mut bytes, ph + 24, SEGMENT_ADDRESS + 0x1000);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentVirtualPhysicalMismatch { .. })
        ));

        let mut bytes = fixture();
        write_u64(&mut bytes, ph + 16, 0x1000);
        write_u64(&mut bytes, ph + 24, 0x1000);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentOverlapsBootstrapPageTables { .. })
        ));
    }

    #[test]
    fn rejects_entry_outside_executable_file_backed_range() {
        let mut bytes = fixture();
        write_u64(&mut bytes, 24, SEGMENT_ADDRESS + 0x170);
        assert_eq!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::EntryNotInExecutableFileBackedSegment {
                entry: SEGMENT_ADDRESS + 0x170,
            })
        );
    }

    #[test]
    fn rejects_invalid_or_incongruent_segment_alignment() {
        let ph = ELF64_HEADER_SIZE;
        let mut bytes = fixture();
        write_u64(&mut bytes, ph + 48, 3);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::InvalidSegmentAlignment { .. })
        ));

        let mut bytes = fixture();
        write_u64(&mut bytes, ph + 8, 1);
        assert!(matches!(
            Elf64GuestImage::parse(&bytes),
            Err(Elf64Error::SegmentAlignmentMismatch { .. })
        ));
    }
}
