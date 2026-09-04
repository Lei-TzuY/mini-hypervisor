use crate::error::Error;
use crate::memory::{GuestMemory, GuestMemoryRegion, GuestPhysAddr};
use std::fmt;

pub const LONG_MODE_IDENTITY_MAP_SIZE: u64 = 2 * 1024 * 1024;
pub const LONG_MODE_PML4_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x1000);
pub const LONG_MODE_PDPT_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x2000);
pub const LONG_MODE_PD_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x3000);
pub const LONG_MODE_PAGE_TABLE_END: GuestPhysAddr = GuestPhysAddr::new(0x4000);
pub const LONG_MODE_CR0_REQUIRED_BITS: u64 = (1 << 0) | (1 << 31);
pub const LONG_MODE_CR4_REQUIRED_BITS: u64 = 1 << 5;
pub const LONG_MODE_EFER_REQUIRED_BITS: u64 = (1 << 8) | (1 << 10);

const PAGE_PRESENT: u64 = 1 << 0;
const PAGE_WRITABLE: u64 = 1 << 1;
const PAGE_SIZE_2_MIB: u64 = 1 << 7;
const PAGE_TABLE_ENTRY_FLAGS: u64 = PAGE_PRESENT | PAGE_WRITABLE;
const LARGE_PAGE_ENTRY_FLAGS: u64 = PAGE_TABLE_ENTRY_FLAGS | PAGE_SIZE_2_MIB;

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
    EntryOverlapsPageTables {
        entry: u64,
    },
    StackPointerOutsideIdentityMap {
        stack_pointer: u64,
        mapped_size: u64,
    },
    StackPointerOverlapsPageTables {
        stack_pointer: u64,
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
            Self::EntryOverlapsPageTables { entry } => write!(
                f,
                "long-mode entry {entry:#x} overlaps the reserved bootstrap page-table pages"
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
        }
    }
}

impl std::error::Error for LongModeConfigurationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LongModeBootLayout {
    memory: GuestMemoryRegion,
    entry: GuestPhysAddr,
    stack_pointer: u64,
}

impl LongModeBootLayout {
    pub fn new(
        memory: GuestMemoryRegion,
        entry: GuestPhysAddr,
        stack_pointer: u64,
    ) -> Result<Self, LongModeConfigurationError> {
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
        if entry.get() >= LONG_MODE_IDENTITY_MAP_SIZE {
            return Err(LongModeConfigurationError::EntryOutsideIdentityMap {
                entry: entry.get(),
                mapped_size: LONG_MODE_IDENTITY_MAP_SIZE,
            });
        }
        if is_page_table_address(entry.get()) {
            return Err(LongModeConfigurationError::EntryOverlapsPageTables { entry: entry.get() });
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

        Ok(Self {
            memory,
            entry,
            stack_pointer,
        })
    }

    #[must_use]
    pub const fn memory(&self) -> GuestMemoryRegion {
        self.memory
    }

    #[must_use]
    pub const fn entry(&self) -> GuestPhysAddr {
        self.entry
    }

    #[must_use]
    pub const fn stack_pointer(&self) -> u64 {
        self.stack_pointer
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
    pub const fn identity_map_size(&self) -> u64 {
        LONG_MODE_IDENTITY_MAP_SIZE
    }

    pub(crate) fn install_page_tables(&self, memory: &mut GuestMemory) -> Result<(), Error> {
        debug_assert_eq!(memory.region(), self.memory);

        let zero_page = [0_u8; 4096];
        memory.write(LONG_MODE_PML4_ADDR, &zero_page)?;
        memory.write(LONG_MODE_PDPT_ADDR, &zero_page)?;
        memory.write(LONG_MODE_PD_ADDR, &zero_page)?;

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

        Ok(())
    }
}

const fn is_page_table_address(address: u64) -> bool {
    address >= LONG_MODE_PML4_ADDR.get() && address < LONG_MODE_PAGE_TABLE_END.get()
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
    fn layout_contract_is_fixed_and_identity_mapped() {
        let layout = layout();
        assert_eq!(layout.memory(), memory_region());
        assert_eq!(layout.entry(), ENTRY);
        assert_eq!(layout.stack_pointer(), STACK);
        assert_eq!(layout.pml4_address(), LONG_MODE_PML4_ADDR);
        assert_eq!(layout.pdpt_address(), LONG_MODE_PDPT_ADDR);
        assert_eq!(layout.pd_address(), LONG_MODE_PD_ADDR);
        assert_eq!(layout.identity_map_size(), 0x20_0000);
        assert_eq!(LONG_MODE_PML4_ADDR.get() % KVM_MEMORY_ALIGNMENT, 0);
        assert_eq!(LONG_MODE_PDPT_ADDR.get() % KVM_MEMORY_ALIGNMENT, 0);
        assert_eq!(LONG_MODE_PD_ADDR.get() % KVM_MEMORY_ALIGNMENT, 0);
    }

    #[test]
    fn installs_minimal_four_level_page_table_chain_with_one_two_mib_page() {
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
            read_u64(&memory, GuestPhysAddr::new(LONG_MODE_PML4_ADDR.get() + 8)),
            0
        );
        assert_eq!(
            read_u64(&memory, GuestPhysAddr::new(LONG_MODE_PDPT_ADDR.get() + 8)),
            0
        );
        assert_eq!(
            read_u64(&memory, GuestPhysAddr::new(LONG_MODE_PD_ADDR.get() + 8)),
            0
        );
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
    fn rejects_entry_outside_identity_map_or_inside_page_tables() {
        assert!(matches!(
            LongModeBootLayout::new(
                memory_region(),
                GuestPhysAddr::new(LONG_MODE_IDENTITY_MAP_SIZE),
                STACK
            ),
            Err(LongModeConfigurationError::EntryOutsideIdentityMap { .. })
        ));
        assert!(matches!(
            LongModeBootLayout::new(memory_region(), LONG_MODE_PDPT_ADDR, STACK),
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
