mod guest;
pub use guest::*;

use super::{
    page_set::page_set_error, BoundedCheckpointPage, BoundedCheckpointPageComparison,
    BoundedPageSetSnapshot, VcpuStateSnapshot, VcpuStateSnapshotComparison,
};
use crate::error::Error;
use crate::kvm::msr::GuestMsrAccessPolicy;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::vcpu::{Vcpu, VcpuId};

pub const BOUNDED_COORDINATED_VCPU_COUNT: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedVcpuStateCheckpoint {
    id: VcpuId,
    snapshot: VcpuStateSnapshot,
}

impl BoundedVcpuStateCheckpoint {
    #[must_use]
    pub const fn id(&self) -> VcpuId {
        self.id
    }

    #[must_use]
    pub const fn snapshot(&self) -> &VcpuStateSnapshot {
        &self.snapshot
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedTwoVcpuPageSetCheckpoint {
    pages: BoundedPageSetSnapshot,
    vcpus: [BoundedVcpuStateCheckpoint; BOUNDED_COORDINATED_VCPU_COUNT],
}

impl BoundedTwoVcpuPageSetCheckpoint {
    pub fn capture(
        vcpus: [&Vcpu; BOUNDED_COORDINATED_VCPU_COUNT],
        msr_policy: &GuestMsrAccessPolicy,
        memory: &GuestMemory,
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let [first, second] = canonical_vcpus(vcpus)?;
        let pages = BoundedPageSetSnapshot::capture(memory, page_addresses)?;
        let first = BoundedVcpuStateCheckpoint {
            id: first.id(),
            snapshot: first.capture_state_snapshot(msr_policy)?,
        };
        let second = BoundedVcpuStateCheckpoint {
            id: second.id(),
            snapshot: second.capture_state_snapshot(msr_policy)?,
        };
        Ok(Self {
            pages,
            vcpus: [first, second],
        })
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        self.pages.pages()
    }

    #[must_use]
    pub fn page(&self, address: GuestPhysAddr) -> Option<&BoundedCheckpointPage> {
        self.pages.page(address)
    }

    #[must_use]
    pub const fn vcpus(&self) -> &[BoundedVcpuStateCheckpoint; BOUNDED_COORDINATED_VCPU_COUNT] {
        &self.vcpus
    }

    #[must_use]
    pub fn vcpu(&self, id: VcpuId) -> Option<&BoundedVcpuStateCheckpoint> {
        self.vcpus
            .binary_search_by_key(&id, BoundedVcpuStateCheckpoint::id)
            .ok()
            .map(|index| &self.vcpus[index])
    }

    pub fn verify(
        &self,
        vcpus: [&Vcpu; BOUNDED_COORDINATED_VCPU_COUNT],
        memory: &GuestMemory,
    ) -> Result<BoundedTwoVcpuPageSetCheckpointComparison, Error> {
        let targets = self.canonical_targets(vcpus)?;
        let pages = self.pages.verify(memory)?;
        let vcpus = [
            BoundedVcpuStateCheckpointComparison {
                id: self.vcpus[0].id,
                comparison: targets[0].verify_state_snapshot(&self.vcpus[0].snapshot)?,
            },
            BoundedVcpuStateCheckpointComparison {
                id: self.vcpus[1].id,
                comparison: targets[1].verify_state_snapshot(&self.vcpus[1].snapshot)?,
            },
        ];
        Ok(BoundedTwoVcpuPageSetCheckpointComparison { pages, vcpus })
    }

    pub fn restore_and_verify(
        &self,
        vcpus: [&Vcpu; BOUNDED_COORDINATED_VCPU_COUNT],
        memory: &mut GuestMemory,
    ) -> Result<BoundedTwoVcpuPageSetCheckpointComparison, Error> {
        let targets = self.canonical_targets(vcpus)?;
        self.pages.restore(memory)?;
        targets[0].restore_state_snapshot(&self.vcpus[0].snapshot)?;
        targets[1].restore_state_snapshot(&self.vcpus[1].snapshot)?;

        let pages = self.pages.verify(memory)?;
        let vcpus = [
            BoundedVcpuStateCheckpointComparison {
                id: self.vcpus[0].id,
                comparison: targets[0].verify_state_snapshot(&self.vcpus[0].snapshot)?,
            },
            BoundedVcpuStateCheckpointComparison {
                id: self.vcpus[1].id,
                comparison: targets[1].verify_state_snapshot(&self.vcpus[1].snapshot)?,
            },
        ];
        Ok(BoundedTwoVcpuPageSetCheckpointComparison { pages, vcpus })
    }

    fn canonical_targets<'a>(
        &self,
        vcpus: [&'a Vcpu; BOUNDED_COORDINATED_VCPU_COUNT],
    ) -> Result<[&'a Vcpu; BOUNDED_COORDINATED_VCPU_COUNT], Error> {
        let targets = canonical_vcpus(vcpus)?;
        for index in 0..BOUNDED_COORDINATED_VCPU_COUNT {
            if targets[index].id() != self.vcpus[index].id {
                return Err(page_set_error(
                    "bounded two-vCPU checkpoint target validation",
                    format!(
                        "checkpoint expects vCPU IDs [{}, {}], got [{}, {}]",
                        self.vcpus[0].id.get(),
                        self.vcpus[1].id.get(),
                        targets[0].id().get(),
                        targets[1].id().get()
                    ),
                ));
            }
        }
        Ok(targets)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedVcpuStateCheckpointComparison {
    id: VcpuId,
    comparison: VcpuStateSnapshotComparison,
}

impl BoundedVcpuStateCheckpointComparison {
    #[must_use]
    pub const fn id(&self) -> VcpuId {
        self.id
    }

    #[must_use]
    pub const fn comparison(&self) -> &VcpuStateSnapshotComparison {
        &self.comparison
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.comparison.is_exact_match()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedTwoVcpuPageSetCheckpointComparison {
    pages: Vec<BoundedCheckpointPageComparison>,
    vcpus: [BoundedVcpuStateCheckpointComparison; BOUNDED_COORDINATED_VCPU_COUNT],
}

impl BoundedTwoVcpuPageSetCheckpointComparison {
    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPageComparison] {
        &self.pages
    }

    #[must_use]
    pub const fn vcpus(
        &self,
    ) -> &[BoundedVcpuStateCheckpointComparison; BOUNDED_COORDINATED_VCPU_COUNT] {
        &self.vcpus
    }

    #[must_use]
    pub fn page_exact(&self, address: GuestPhysAddr) -> Option<bool> {
        self.pages
            .binary_search_by_key(&address.get(), |page| page.address().get())
            .ok()
            .map(|index| self.pages[index].exact())
    }

    #[must_use]
    pub fn vcpu_exact(&self, id: VcpuId) -> Option<bool> {
        self.vcpus
            .binary_search_by_key(&id, BoundedVcpuStateCheckpointComparison::id)
            .ok()
            .map(|index| self.vcpus[index].is_exact_match())
    }

    #[must_use]
    pub fn all_pages_exact(&self) -> bool {
        self.pages.iter().all(|page| page.exact())
    }

    #[must_use]
    pub fn all_vcpus_exact(&self) -> bool {
        self.vcpus
            .iter()
            .all(BoundedVcpuStateCheckpointComparison::is_exact_match)
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.all_pages_exact() && self.all_vcpus_exact()
    }
}

fn canonical_vcpus(
    mut vcpus: [&Vcpu; BOUNDED_COORDINATED_VCPU_COUNT],
) -> Result<[&Vcpu; BOUNDED_COORDINATED_VCPU_COUNT], Error> {
    vcpus.sort_unstable_by_key(|vcpu| vcpu.id());
    if vcpus[0].id() == vcpus[1].id() {
        return Err(page_set_error(
            "bounded two-vCPU checkpoint capture",
            format!("duplicate vCPU ID {}", vcpus[0].id().get()),
        ));
    }
    Ok(vcpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinated_checkpoint_count_is_exactly_two() {
        assert_eq!(BOUNDED_COORDINATED_VCPU_COUNT, 2);
    }
}
