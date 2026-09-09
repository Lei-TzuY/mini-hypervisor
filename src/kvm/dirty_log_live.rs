#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActiveDirtyLogSlot0 {
    slot: DirtyLogSlot0,
}

impl ActiveDirtyLogSlot0 {
    #[must_use]
    pub(crate) const fn page_count(self) -> u64 {
        self.slot.page_count
    }
}

impl crate::kvm::Vm {
    pub(crate) fn enable_dirty_log_slot0(
        &mut self,
    ) -> Result<ActiveDirtyLogSlot0, crate::error::Error> {
        let memory = self.guest_memory.as_ref().ok_or_else(|| {
            dirty_log_vm_error(
                "enable slot-0 dirty logging",
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "dirty logging requires registered guest memory",
                ),
            )
        })?;
        super::validate_guest_memory_registration(memory.region())?;
        let region = memory.region();
        let request = KvmUserspaceMemoryRegion {
            slot: DIRTY_LOG_SLOT,
            flags: KVM_MEM_LOG_DIRTY_PAGES,
            guest_phys_addr: region.base().get(),
            memory_size: region.size(),
            userspace_addr: memory.userspace_addr(),
        };
        set_user_memory_region(std::os::fd::AsRawFd::as_raw_fd(&self.fd), &request).map_err(
            |source| dirty_log_vm_error("enable slot-0 dirty logging", source),
        )?;
        Ok(ActiveDirtyLogSlot0 {
            slot: DirtyLogSlot0::for_region(region),
        })
    }

    pub(crate) fn harvest_dirty_log_slot0(
        &self,
        active: ActiveDirtyLogSlot0,
    ) -> Result<Vec<u64>, crate::error::Error> {
        harvest_dirty_log(self, active.slot)
    }
}

#[cfg(test)]
mod live_dirty_log_tests {
    use super::*;

    #[test]
    fn active_slot_token_retains_region_page_count() {
        let region = crate::memory::GuestMemoryRegion::new(
            crate::memory::GuestPhysAddr::new(0),
            8 * crate::memory::KVM_MEMORY_ALIGNMENT,
        )
        .unwrap();
        let active = ActiveDirtyLogSlot0 {
            slot: DirtyLogSlot0::for_region(region),
        };
        assert_eq!(active.page_count(), 8);
    }
}
