#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Slot0DirtyTracker {
    slot: DirtyLogSlot0,
    region: crate::memory::GuestMemoryRegion,
}

impl Slot0DirtyTracker {
    #[must_use]
    pub(crate) const fn page_count(self) -> u64 {
        self.slot.page_count
    }

    pub(crate) fn page_index(
        self,
        address: crate::memory::GuestPhysAddr,
    ) -> Result<u64, crate::error::Error> {
        let start = self.region.base().get();
        let end = self.region.end().get();
        if address.get() < start || address.get() >= end {
            return Err(dirty_log_verification_error(
                "slot-0 dirty-page lookup",
                format!(
                    "guest address {:#x} is outside tracked region {:#x}..{:#x}",
                    address.get(),
                    start,
                    end
                ),
            ));
        }
        Ok((address.get() - start) / crate::memory::KVM_MEMORY_ALIGNMENT)
    }

    pub(crate) fn bitmap_contains_page(
        self,
        bitmap: &[u64],
        address: crate::memory::GuestPhysAddr,
    ) -> Result<bool, crate::error::Error> {
        let page = self.page_index(address)?;
        let word = usize::try_from(page / 64).map_err(|_| {
            dirty_log_verification_error(
                "slot-0 dirty-page lookup",
                format!("dirty page index {page} does not fit host usize"),
            )
        })?;
        let bit = u32::try_from(page % 64).expect("dirty bitmap bit is always below 64");
        let value = bitmap.get(word).ok_or_else(|| {
            dirty_log_verification_error(
                "slot-0 dirty-page lookup",
                format!(
                    "dirty bitmap has {} words but page {page} requires word {word}",
                    bitmap.len()
                ),
            )
        })?;
        Ok(value & (1_u64 << bit) != 0)
    }
}

impl crate::kvm::Vm {
    pub(crate) fn register_guest_memory_with_dirty_tracking(
        &mut self,
        memory: crate::memory::GuestMemory,
    ) -> Result<Slot0DirtyTracker, crate::error::Error> {
        let region = memory.region();
        let slot = register_guest_memory_with_dirty_log(self, memory)?;
        Ok(Slot0DirtyTracker { slot, region })
    }

    pub(crate) fn harvest_slot0_dirty(
        &self,
        tracker: Slot0DirtyTracker,
    ) -> Result<Vec<u64>, crate::error::Error> {
        harvest_dirty_log(self, tracker.slot)
    }
}

#[cfg(test)]
mod task_dirty_tests {
    use super::*;

    #[test]
    fn tracker_maps_guest_pages_to_bitmap_bits() {
        let region = crate::memory::GuestMemoryRegion::new(
            crate::memory::GuestPhysAddr::new(0),
            128 * crate::memory::KVM_MEMORY_ALIGNMENT,
        )
        .unwrap();
        let tracker = Slot0DirtyTracker {
            slot: DirtyLogSlot0::for_region(region),
            region,
        };
        let bitmap = [1_u64 << 48, 1_u64 << 3];
        assert_eq!(tracker.page_count(), 128);
        assert_eq!(
            tracker
                .page_index(crate::memory::GuestPhysAddr::new(0x30000))
                .unwrap(),
            48
        );
        assert!(tracker
            .bitmap_contains_page(&bitmap, crate::memory::GuestPhysAddr::new(0x30000))
            .unwrap());
        assert!(tracker
            .bitmap_contains_page(&bitmap, crate::memory::GuestPhysAddr::new(0x43000))
            .unwrap());
        assert!(!tracker
            .bitmap_contains_page(&bitmap, crate::memory::GuestPhysAddr::new(0x44000))
            .unwrap());
    }

    #[test]
    fn tracker_rejects_addresses_outside_slot0() {
        let region = crate::memory::GuestMemoryRegion::new(
            crate::memory::GuestPhysAddr::new(0x1000),
            2 * crate::memory::KVM_MEMORY_ALIGNMENT,
        )
        .unwrap();
        let tracker = Slot0DirtyTracker {
            slot: DirtyLogSlot0::for_region(region),
            region,
        };
        assert!(tracker
            .page_index(crate::memory::GuestPhysAddr::new(0))
            .is_err());
        assert!(tracker
            .page_index(crate::memory::GuestPhysAddr::new(0x3000))
            .is_err());
    }
}
