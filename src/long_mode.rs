use crate::error::Error;
use crate::memory::{GuestMemory, GuestMemoryRegion, GuestPhysAddr};
use std::fmt;

pub const LONG_MODE_PAGE_SIZE: u64 = 4096;
pub const LONG_MODE_IDENTITY_MAP_SIZE: u64 = 2 * 1024 * 1024;
pub const LONG_MODE_ALIAS_VIRTUAL_BASE: u64 = 0x40_0000;
pub const LONG_MODE_ALIAS_VIRTUAL_SIZE: u64 = 2 * 1024 * 1024;
pub const LONG_MODE_ALIAS_VIRTUAL_END: u64 =
    LONG_MODE_ALIAS_VIRTUAL_BASE + LONG_MODE_ALIAS_VIRTUAL_SIZE;
pub const LONG_MODE_PML4_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x1000);
pub const LONG_MODE_PDPT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x2000);
pub const LONG_MODE_PD_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x3000);
pub const LONG_MODE_ALIAS_PT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x4000);
pub const LONG_MODE_PAGE_TABLE_END: GuestPhysAddr = GuestPhysAddr::new(0x5000);
pub const LONG_MODE_CR0_REQUIRED_BITS: u64 = (1 << 0) | (1 << 31);
pub const LONG_MODE_CR4_REQUIRED_BITS: u64 = 1 << 5;
pub const LONG_MODE_EFER_REQUIRED_BITS: u64 = (1 << 8) | (1 << 10);

const PAGE_PRESENT: u64 = 1 << 0;
const PAGE_WRITABLE: u64 = 1 << 1;
const PAGE_SIZE_2_MIB: u64 = 1 << 7;
const PAGE_TABLE_ENTRY_FLAGS: u64 = PAGE_PRESENT | PAGE_WRITABLE;
const LARGE_PAGE_ENTRY_FLAGS: u64 = PAGE_TABLE_ENTRY_FLAGS | PAGE_SIZE_2_MIB;
const ALIAS_PD_INDEX: u64 = LONG_MODE_ALIAS_VIRTUAL_BASE / LONG_MODE_IDENTITY_MAP_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LongModePageMapping {
    virtual_page: u64,
    physical_page: GuestPhysAddr,
}

impl LongModePageMapping {
    #[must_use]
    pub const fn new(virtual_page: u64, physical_page: GuestPhysAddr) -> Self {
        Self {
            virtual_page,
            physical_page,
        }
    }

    #[must_use]
    pub const fn virtual_page(self) -> u64 {
        self.virtual_page
    }

    #[must_use]
    pub const fn physical_page(self) -> GuestPhysAddr {
        self.physical_page
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LongModeConfigurationError {
    RamMustStartAtZero {
        base: u64,
    },
    RamTooSmall {
        size: u64,
        minimum: u64,
    },
    EntryOutsideIdentityMap {
        entry: u64,
        mapped_size: u64,
    },
    EntryOutsideMappedRanges {
        entry: u64,
    },
    EntryOverlapsPageTables {
        entry: u64,
    },
    EntryPageNotMapped {
        entry: u64,
        virtual_page: u64,
    },
    StackPointerOutsideIdentityMap {
        stack_pointer: u64,
        mapped_size: u64,
    },
    StackPointerOverlapsPageTables {
        stack_pointer: u64,
    },
    MappingVirtualPageMisaligned {
        virtual_page: u64,
    },
    MappingOutsideAliasWindow {
        virtual_page: u64,
    },
    MappingPhysicalPageMisaligned {
        physical_page: u64,
    },
    MappingPhysicalPageOutsideRam {
        physical_page: u64,
    },
    MappingPhysicalPageOverlapsPageTables {
        physical_page: u64,
    },
    DuplicateVirtualPage {
        virtual_page: u64,
    },
}

impl fmt::Display for LongModeConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RamMustStartAtZero { base } => write!(
                f,
                "long-mode bootstrap requires guest RAM to start at physical address 0, got {base:#x}"
            ),
            Self::RamTooSmall { size, minimum } => write!(
                f,
                "long-mode bootstrap requires at least {minimum:#x} bytes of guest RAM, got {size:#x}"
            ),
            Self::EntryOutsideIdentityMap { entry, mapped_size } => write!(
                f,
                "long-mode entry {entry:#x} is outside the identity-mapped range 0..{mapped_size:#x}"
            ),
            Self::EntryOutsideMappedRanges { entry } => write!(
                f,
                "long-mode entry {entry:#x} is outside the identity map and bounded alias window"
            ),
            Self::EntryOverlapsPageTables { entry } => write!(
                f,
                "long-mode entry {entry:#x} overlaps the reserved bootstrap page-table pages"
            ),
            Self::EntryPageNotMapped {
                entry,
                virtual_page,
            } => write!(
                f,
                "long-mode entry {entry:#x} lies on unmapped alias page {virtual_page:#x}"
            ),
            Self::StackPointerOutsideIdentityMap {
                stack_pointer,
                mapped_size,
            } => write!(
                f,
                "long-mode stack pointer {stack_pointer:#x} is outside the identity-mapped range 0..{mapped_size:#x}"
            ),
            Self::StackPointerOverlapsPageTables { stack_pointer } => write!(
                f,
                "long-mode stack pointer {stack_pointer:#x} overlaps the reserved bootstrap page-table pages"
            ),
            Self::MappingVirtualPageMisaligned { virtual_page } => write!(
                f,
                "long-mode alias virtual page {virtual_page:#x} is not 4 KiB aligned"
            ),
            Self::MappingOutsideAliasWindow { virtual_page } => write!(
                f,
                "long-mode alias virtual page {virtual_page:#x} is outside {LONG_MODE_ALIAS_VIRTUAL_BASE:#x}..{LONG_MODE_ALIAS_VIRTUAL_END:#x}"
            ),
            Self::MappingPhysicalPageMisaligned { physical_page } => write!(
                f,
                "long-mode alias physical page {physical_page:#x} is not 4 KiB aligned"
            ),
            Self::MappingPhysicalPageOutsideRam { physical_page } => write!(
                f,
                "long-mode alias physical page {physical_page:#x} is outside the low 2 MiB backing range"
            ),
            Self::MappingPhysicalPageOverlapsPageTables { physical_page } => write!(
                f,
                "long-mode alias physical page {physical_page:#x} overlaps bootstrap page tables"
            ),
            Self::DuplicateVirtualPage { virtual_page } => write!(
                f,
                "long-mode alias virtual page {virtual_page:#x} is mapped more than once"
            ),
        }
    }
}

impl std::error::Error for LongModeConfigurationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongModeBootLayout {
    memory: GuestMemoryRegion,
    entry: u64,
    stack_pointer: u64,
    page_mappings: Vec<LongModePageMapping>,
}

impl LongModeBootLayout {
    pub fn new(
        memory: GuestMemoryRegion,
        entry: GuestPhysAddr,
        stack_pointer: u64,
    ) -> Result<Self, LongModeConfigurationError> {
        validate_memory_and_stack(memory, stack_pointer)?;
        if entry.get() >= LONG_MODE_IDENTITY_MAP_SIZE {
            return Err(LongModeConfigurationError::EntryOutsideIdentityMap {
                entry: entry.get(),
                mapped_size: LONG_MODE_IDENTITY_MAP_SIZE,
            });
        }
        if is_page_table_address(entry.get()) {
            return Err(LongModeConfigurationError::EntryOverlapsPageTables { entry: entry.get() });
        }

        Ok(Self {
            memory,
            entry: entry.get(),
            stack_pointer,
            page_mappings: Vec::new(),
        })
    }

    pub fn with_page_mappings(
        memory: GuestMemoryRegion,
        entry: u64,
        stack_pointer: u64,
        page_mappings: Vec<LongModePageMapping>,
    ) -> Result<Self, LongModeConfigurationError> {
        validate_memory_and_stack(memory, stack_pointer)?;
        validate_page_mappings(&page_mappings)?;

        if entry < LONG_MODE_IDENTITY_MAP_SIZE {
            if is_page_table_address(entry) {
                return Err(LongModeConfigurationError::EntryOverlapsPageTables { entry });
            }
        } else if (LONG_MODE_ALIAS_VIRTUAL_BASE..LONG_MODE_ALIAS_VIRTUAL_END).contains(&entry) {
            let virtual_page = align_down_page(entry);
            if !page_mappings
                .iter()
                .any(|mapping| mapping.virtual_page == virtual_page)
            {
                return Err(LongModeConfigurationError::EntryPageNotMapped {
                    entry,
                    virtual_page,
                });
            }
        } else {
            return Err(LongModeConfigurationError::EntryOutsideMappedRanges { entry });
        }

        Ok(Self {
            memory,
            entry,
            stack_pointer,
            page_mappings,
        })
    }

    #[must_use]
    pub const fn memory(&self) -> GuestMemoryRegion {
        self.memory
    }

    #[must_use]
    pub const fn entry(&self) -> u64 {
        self.entry
    }

    #[must_use]
    pub const fn stack_pointer(&self) -> u64 {
        self.stack_pointer
    }

    #[must_use]
    pub fn page_mappings(&self) -> &[LongModePageMapping] {
        &self.page_mappings
    }

    #[must_use]
    pub const fn pml4_address(&self) -> GuestPhysAddr {
        LONG_MODE_PML4_ADDR
    }

    #[must_use]
    pub const fn pdpt_address(&self) -> GuestPhysAddr {
        LONG_MODE_PDPT_ADDR
    }

    #[must_use]
    pub const fn pd_address(&self) -> GuestPhysAddr {
        LONG_MODE_PD_ADDR
    }

    #[must_use]
    pub const fn alias_pt_address(&self) -> GuestPhysAddr {
        LONG_MODE_ALIAS_PT_ADDR
    }

    #[must_use]
    pub const fn identity_map_size(&self) -> u64 {
        LONG_MODE_IDENTITY_MAP_SIZE
    }

    pub(crate) fn install_page_tables(&self, memory: &mut GuestMemory) -> Result<(), Error> {
        debug_assert_eq!(memory.region(), self.memory);

        let zero_page = [0_u8; LONG_MODE_PAGE_SIZE as usize];
        memory.write(LONG_MODE_PML4_ADDR, &zero_page)?;
        memory.write(LONG_MODE_PDPT_ADDR, &zero_page)?;
        memory.write(LONG_MODE_PD_ADDR, &zero_page)?;
        memory.write(LONG_MODE_ALIAS_PT_ADDR, &zero_page)?;

        write_u64(
            memory,
            LONG_MODE_PML4_ADDR,
            LONG_MODE_PDPT_ADDR.get() | PAGE_TABLE_ENTRY_FLAGS,
        )?;
        write_u64(
            memory,
            LONG_MODE_PDPT_ADDR,
            LONG_MODE_PD_ADDR.get() | PAGE_TABLE_ENTRY_FLAGS,
        )?;
        write_u64(memory, LONG_MODE_PD_ADDR, LARGE_PAGE_ENTRY_FLAGS)?;

        if !self.page_mappings.is_empty() {
            write_u64(
                memory,
                GuestPhysAddr::new(LONG_MODE_PD_ADDR.get() + ALIAS_PD_INDEX * 8),
                LONG_MODE_ALIAS_PT_ADDR.get() | PAGE_TABLE_ENTRY_FLAGS,
            )?;
            for mapping in &self.page_mappings {
                let index = (mapping.virtual_page - LONG_MODE_ALIAS_VIRTUAL_BASE)
                    / LONG_MODE_PAGE_SIZE;
                write_u64(
                    memory,
                    GuestPhysAddr::new(LONG_MODE_ALIAS_PT_ADDR.get() + index * 8),
                    mapping.physical_page.get() | PAGE_TABLE_ENTRY_FLAGS,
                )?;
            }
        }

        Ok(())
    }
}

fn validate_memory_and_stack(
    memory: GuestMemoryRegion,
    stack_pointer: u64,
) -> Result<(), LongModeConfigurationError> {
    if memory.base().get() != 0 {
        return Err(LongModeConfigurationError::RamMustStartAtZero {
            base: memory.base().get(),
        });
    }
    if memory.size() < LONG_MODE_IDENTITY_MAP_SIZE {
        return Err(LongModeConfigurationError::RamTooSmall {
            size: memory.size(),
            minimum: LONG_MODE_IDENTITY_MAP_SIZE,
        });
    }
    if stack_pointer == 0 || stack_pointer > LONG_MODE_IDENTITY_MAP_SIZE {
        return Err(LongModeConfigurationError::StackPointerOutsideIdentityMap {
            stack_pointer,
            mapped_size: LONG_MODE_IDENTITY_MAP_SIZE,
        });
    }
    if stack_pointer > LONG_MODE_PML4_ADDR.get()
        && stack_pointer <= LONG_MODE_PAGE_TABLE_END.get()
    {
        return Err(LongModeConfigurationError::StackPointerOverlapsPageTables {
            stack_pointer,
        });
    }
    Ok(())
}

fn validate_page_mappings(
    page_mappings: &[LongModePageMapping],
) -> Result<(), LongModeConfigurationError> {
    for (index, mapping) in page_mappings.iter().enumerate() {
        if mapping.virtual_page % LONG_MODE_PAGE_SIZE != 0 {
            return Err(LongModeConfigurationError::MappingVirtualPageMisaligned {
                virtual_page: mapping.virtual_page,
            });
        }
        if mapping.virtual_page < LONG_MODE_ALIAS_VIRTUAL_BASE
            || mapping.virtual_page >= LONG_MODE_ALIAS_VIRTUAL_END
        {
            return Err(LongModeConfigurationError::MappingOutsideAliasWindow {
                virtual_page: mapping.virtual_page,
            });
        }
        if mapping.physical_page.get() % LONG_MODE_PAGE_SIZE != 0 {
            return Err(LongModeConfigurationError::MappingPhysicalPageMisaligned {
                physical_page: mapping.physical_page.get(),
            });
        }
        let Some(physical_end) = mapping
            .physical_page
            .get()
            .checked_add(LONG_MODE_PAGE_SIZE)
        else {
            return Err(LongModeConfigurationError::MappingPhysicalPageOutsideRam {
                physical_page: mapping.physical_page.get(),
            });
        };
        if physical_end > LONG_MODE_IDENTITY_MAP_SIZE {
            return Err(LongModeConfigurationError::MappingPhysicalPageOutsideRam {
                physical_page: mapping.physical_page.get(),
            });
        }
        if ranges_overlap(
            mapping.physical_page.get(),
            physical_end,
            LONG_MODE_PML4_ADDR.get(),
            LONG_MODE_PAGE_TABLE_END.get(),
        ) {
            return Err(
                LongModeConfigurationError::MappingPhysicalPageOverlapsPageTables {
                    physical_page: mapping.physical_page.get(),
                },
            );
        }
        if page_mappings[..index]
            .iter()
            .any(|previous| previous.virtual_page == mapping.virtual_page)
        {
            return Err(LongModeConfigurationError::DuplicateVirtualPage {
                virtual_page: mapping.virtual_page,
            });
        }
    }
    Ok(())
}

const fn align_down_page(address: u64) -> u64 {
    address & !(LONG_MODE_PAGE_SIZE - 1)
}

const fn is_page_table_address(address: u64) -> bool {
    address >= LONG_MODE_PML4_ADDR.get() && address < LONG_MODE_PAGE_TABLE_END.get()
}

const fn ranges_overlap(
    first_start: u64,
    first_end: u64,
    second_start: u64,
    second_end: u64,
) -> bool {
    first_start < second_end && second_start < first_end
}

fn write_u64(memory: &mut GuestMemory, address: GuestPhysAddr, value: u64) -> Result<(), Error> {
    memory.write(address, &value.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::KVM_MEMORY_ALIGNMENT;

    const ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x1_0000);
    const STACK: u64 = 0x1f_f000;

    fn memory_region() -> GuestMemoryRegion {
        GuestMemoryRegion::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap()
    }

    fn layout() -> LongModeBootLayout {
        LongModeBootLayout::new(memory_region(), ENTRY, STACK).unwrap()
    }

    fn read_u64(memory: &GuestMemory, address: GuestPhysAddr) -> u64 {
        let mut bytes = [0_u8; 8];
        memory.read(address, &mut bytes).unwrap();
        u64::from_le_bytes(bytes)
    }

    #[test]
    fn layout_contract_preserves_low_identity_map() {
        let layout = layout();
        assert_eq!(layout.memory(), memory_region());
        assert_eq!(layout.entry(), ENTRY.get());
        assert_eq!(layout.stack_pointer(), STACK);
        assert!(layout.page_mappings().is_empty());
        assert_eq!(layout.pml4_address(), LONG_MODE_PML4_ADDR);
        assert_eq!(layout.pdpt_address(), LONG_MODE_PDPT_ADDR);
        assert_eq!(layout.pd_address(), LONG_MODE_PD_ADDR);
        assert_eq!(layout.alias_pt_address(), LONG_MODE_ALIAS_PT_ADDR);
        assert_eq!(layout.identity_map_size(), 0x20_0000);
        assert_eq!(LONG_MODE_PML4_ADDR.get() % KVM_MEMORY_ALIGNMENT, 0);
        assert_eq!(LONG_MODE_PDPT_ADDR.get() % KVM_MEMORY_ALIGNMENT, 0);
        assert_eq!(LONG_MODE_PD_ADDR.get() % KVM_MEMORY_ALIGNMENT, 0);
        assert_eq!(LONG_MODE_ALIAS_PT_ADDR.get() % KVM_MEMORY_ALIGNMENT, 0);
    }

    #[test]
    fn installs_identity_map_without_linking_unused_alias_table() {
        let layout = layout();
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        layout.install_page_tables(&mut memory).unwrap();

        assert_eq!(
            read_u64(&memory, LONG_MODE_PML4_ADDR),
            LONG_MODE_PDPT_ADDR.get() | PAGE_TABLE_ENTRY_FLAGS
        );
        assert_eq!(
            read_u64(&memory, LONG_MODE_PDPT_ADDR),
            LONG_MODE_PD_ADDR.get() | PAGE_TABLE_ENTRY_FLAGS
        );
        assert_eq!(read_u64(&memory, LONG_MODE_PD_ADDR), LARGE_PAGE_ENTRY_FLAGS);
        assert_eq!(
            read_u64(
                &memory,
                GuestPhysAddr::new(LONG_MODE_PD_ADDR.get() + ALIAS_PD_INDEX * 8)
            ),
            0
        );
        assert_eq!(read_u64(&memory, LONG_MODE_ALIAS_PT_ADDR), 0);
    }

    #[test]
    fn installs_bounded_nonidentity_alias_mapping() {
        let mapping = LongModePageMapping::new(
            LONG_MODE_ALIAS_VIRTUAL_BASE,
            GuestPhysAddr::new(0x1_0000),
        );
        let layout = LongModeBootLayout::with_page_mappings(
            memory_region(),
            LONG_MODE_ALIAS_VIRTUAL_BASE + 0x123,
            STACK,
            vec![mapping],
        )
        .unwrap();
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        layout.install_page_tables(&mut memory).unwrap();

        assert_eq!(layout.entry(), LONG_MODE_ALIAS_VIRTUAL_BASE + 0x123);
        assert_eq!(layout.page_mappings(), &[mapping]);
        assert_eq!(
            read_u64(
                &memory,
                GuestPhysAddr::new(LONG_MODE_PD_ADDR.get() + ALIAS_PD_INDEX * 8)
            ),
            LONG_MODE_ALIAS_PT_ADDR.get() | PAGE_TABLE_ENTRY_FLAGS
        );
        assert_eq!(
            read_u64(&memory, LONG_MODE_ALIAS_PT_ADDR),
            0x1_0000 | PAGE_TABLE_ENTRY_FLAGS
        );
    }

    #[test]
    fn rejects_unmapped_alias_entry_and_invalid_mapping_pages() {
        assert!(matches!(
            LongModeBootLayout::with_page_mappings(
                memory_region(),
                LONG_MODE_ALIAS_VIRTUAL_BASE,
                STACK,
                vec![]
            ),
            Err(LongModeConfigurationError::EntryPageNotMapped { .. })
        ));
        assert!(matches!(
            LongModeBootLayout::with_page_mappings(
                memory_region(),
                LONG_MODE_ALIAS_VIRTUAL_BASE,
                STACK,
                vec![LongModePageMapping::new(
                    LONG_MODE_ALIAS_VIRTUAL_BASE + 1,
                    GuestPhysAddr::new(0x1_0000)
                )]
            ),
            Err(LongModeConfigurationError::MappingVirtualPageMisaligned { .. })
        ));
        assert!(matches!(
            LongModeBootLayout::with_page_mappings(
                memory_region(),
                LONG_MODE_ALIAS_VIRTUAL_BASE,
                STACK,
                vec![LongModePageMapping::new(
                    LONG_MODE_ALIAS_VIRTUAL_BASE,
                    GuestPhysAddr::new(0x1_0001)
                )]
            ),
            Err(LongModeConfigurationError::MappingPhysicalPageMisaligned { .. })
        ));
        assert!(matches!(
            LongModeBootLayout::with_page_mappings(
                memory_region(),
                LONG_MODE_ALIAS_VIRTUAL_BASE,
                STACK,
                vec![LongModePageMapping::new(
                    LONG_MODE_ALIAS_VIRTUAL_BASE,
                    LONG_MODE_ALIAS_PT_ADDR
                )]
            ),
            Err(LongModeConfigurationError::MappingPhysicalPageOverlapsPageTables { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_alias_virtual_pages() {
        let mapping = LongModePageMapping::new(
            LONG_MODE_ALIAS_VIRTUAL_BASE,
            GuestPhysAddr::new(0x1_0000),
        );
        assert!(matches!(
            LongModeBootLayout::with_page_mappings(
                memory_region(),
                LONG_MODE_ALIAS_VIRTUAL_BASE,
                STACK,
                vec![mapping, mapping]
            ),
            Err(LongModeConfigurationError::DuplicateVirtualPage { .. })
        ));
    }

    #[test]
    fn rejects_ram_that_does_not_start_at_zero() {
        let region = GuestMemoryRegion::new(
            GuestPhysAddr::new(KVM_MEMORY_ALIGNMENT),
            LONG_MODE_IDENTITY_MAP_SIZE,
        )
        .unwrap();
        assert!(matches!(
            LongModeBootLayout::new(region, ENTRY, STACK),
            Err(LongModeConfigurationError::RamMustStartAtZero { .. })
        ));
    }

    #[test]
    fn rejects_ram_smaller_than_identity_map() {
        let region = GuestMemoryRegion::new(
            GuestPhysAddr::new(0),
            LONG_MODE_IDENTITY_MAP_SIZE - KVM_MEMORY_ALIGNMENT,
        )
        .unwrap();
        assert!(matches!(
            LongModeBootLayout::new(region, ENTRY, STACK),
            Err(LongModeConfigurationError::RamTooSmall { .. })
        ));
    }

    #[test]
    fn rejects_identity_entry_outside_map_or_inside_page_tables() {
        assert!(matches!(
            LongModeBootLayout::new(
                memory_region(),
                GuestPhysAddr::new(LONG_MODE_IDENTITY_MAP_SIZE),
                STACK
            ),
            Err(LongModeConfigurationError::EntryOutsideIdentityMap { .. })
        ));
        assert!(matches!(
            LongModeBootLayout::new(memory_region(), LONG_MODE_ALIAS_PT_ADDR, STACK),
            Err(LongModeConfigurationError::EntryOverlapsPageTables { .. })
        ));
    }

    #[test]
    fn rejects_stack_outside_identity_map_or_inside_page_tables() {
        assert!(matches!(
            LongModeBootLayout::new(memory_region(), ENTRY, 0),
            Err(LongModeConfigurationError::StackPointerOutsideIdentityMap { .. })
        ));
        assert!(matches!(
            LongModeBootLayout::new(memory_region(), ENTRY, LONG_MODE_IDENTITY_MAP_SIZE + 1),
            Err(LongModeConfigurationError::StackPointerOutsideIdentityMap { .. })
        ));
        assert!(matches!(
            LongModeBootLayout::new(memory_region(), ENTRY, LONG_MODE_PAGE_TABLE_END.get()),
            Err(LongModeConfigurationError::StackPointerOverlapsPageTables { .. })
        ));
    }
}
