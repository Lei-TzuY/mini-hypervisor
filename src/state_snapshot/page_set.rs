mod guest;
pub use guest::*;

use super::{VcpuStateSnapshot, VcpuStateSnapshotComparison};
use crate::error::{Error, HostEnvironmentError};
use crate::kvm::msr::GuestMsrAccessPolicy;
use crate::long_mode::LONG_MODE_PAGE_SIZE;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::vcpu::{Vcpu, VcpuId};
use std::io;

pub const BOUNDED_CHECKPOINT_PAGE_SET_LIMIT: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedCheckpointPage {
    address: GuestPhysAddr,
    bytes: Vec<u8>,
}

impl BoundedCheckpointPage {
    #[must_use]
    pub const fn address(&self) -> GuestPhysAddr {
        self.address
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedVcpuPageSetCheckpoint {
    pages: Vec<BoundedCheckpointPage>,
    vcpu: VcpuStateSnapshot,
}

impl BoundedVcpuPageSetCheckpoint {
    pub fn capture(
        vcpu: &Vcpu,
        msr_policy: &GuestMsrAccessPolicy,
        memory: &GuestMemory,
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let page_addresses = validate_page_addresses(page_addresses)?;
        let mut pages = Vec::with_capacity(page_addresses.len());
        for address in page_addresses {
            let mut bytes = vec![0_u8; LONG_MODE_PAGE_SIZE as usize];
            memory.read(address, &mut bytes)?;
            pages.push(BoundedCheckpointPage { address, bytes });
        }
        let vcpu = vcpu.capture_state_snapshot(msr_policy)?;
        Ok(Self { pages, vcpu })
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        &self.pages
    }

    #[must_use]
    pub fn page(&self, address: GuestPhysAddr) -> Option<&BoundedCheckpointPage> {
        self.pages
            .binary_search_by_key(&address.get(), |page| page.address.get())
            .ok()
            .map(|index| &self.pages[index])
    }

    #[must_use]
    pub const fn vcpu(&self) -> &VcpuStateSnapshot {
        &self.vcpu
    }

    pub fn verify(
        &self,
        vcpu: &Vcpu,
        memory: &GuestMemory,
    ) -> Result<BoundedPageSetCheckpointComparison, Error> {
        let pages = compare_pages(&self.pages, memory)?;
        let vcpu = vcpu.verify_state_snapshot(&self.vcpu)?;
        Ok(BoundedPageSetCheckpointComparison { pages, vcpu })
    }

    pub fn restore_and_verify(
        &self,
        vcpu: &Vcpu,
        memory: &mut GuestMemory,
    ) -> Result<BoundedPageSetCheckpointComparison, Error> {
        for page in &self.pages {
            memory.write(page.address, &page.bytes)?;
        }
        let vcpu = vcpu.restore_and_verify_state_snapshot(&self.vcpu)?;
        let pages = compare_pages(&self.pages, memory)?;
        Ok(BoundedPageSetCheckpointComparison { pages, vcpu })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedCheckpointPageComparison {
    address: GuestPhysAddr,
    exact: bool,
}

impl BoundedCheckpointPageComparison {
    #[must_use]
    pub const fn address(self) -> GuestPhysAddr {
        self.address
    }

    #[must_use]
    pub const fn exact(self) -> bool {
        self.exact
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedPageSetCheckpointComparison {
    pages: Vec<BoundedCheckpointPageComparison>,
    vcpu: VcpuStateSnapshotComparison,
}

impl BoundedPageSetCheckpointComparison {
    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPageComparison] {
        &self.pages
    }

    #[must_use]
    pub const fn vcpu(&self) -> &VcpuStateSnapshotComparison {
        &self.vcpu
    }

    #[must_use]
    pub fn all_pages_exact(&self) -> bool {
        self.pages.iter().all(|page| page.exact)
    }

    #[must_use]
    pub fn page_exact(&self, address: GuestPhysAddr) -> Option<bool> {
        self.pages
            .binary_search_by_key(&address.get(), |page| page.address.get())
            .ok()
            .map(|index| self.pages[index].exact)
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.all_pages_exact() && self.vcpu.is_exact_match()
    }
}

fn compare_pages(
    expected: &[BoundedCheckpointPage],
    memory: &GuestMemory,
) -> Result<Vec<BoundedCheckpointPageComparison>, Error> {
    let mut comparisons = Vec::with_capacity(expected.len());
    for page in expected {
        let mut observed = vec![0_u8; page.bytes.len()];
        memory.read(page.address, &mut observed)?;
        comparisons.push(BoundedCheckpointPageComparison {
            address: page.address,
            exact: observed == page.bytes,
        });
    }
    Ok(comparisons)
}

fn validate_page_addresses(page_addresses: &[GuestPhysAddr]) -> Result<Vec<GuestPhysAddr>, Error> {
    if page_addresses.is_empty() {
        return Err(page_set_error(
            "bounded page-set checkpoint capture",
            "checkpoint ownership set must contain at least one page",
        ));
    }
    if page_addresses.len() > BOUNDED_CHECKPOINT_PAGE_SET_LIMIT {
        return Err(page_set_error(
            "bounded page-set checkpoint capture",
            format!(
                "checkpoint ownership set has {} pages, limit is {}",
                page_addresses.len(),
                BOUNDED_CHECKPOINT_PAGE_SET_LIMIT
            ),
        ));
    }

    let mut canonical = page_addresses.to_vec();
    canonical.sort_unstable_by_key(|address| address.get());
    for address in &canonical {
        if address.get() % LONG_MODE_PAGE_SIZE != 0 {
            return Err(page_set_error(
                "bounded page-set checkpoint capture",
                format!("checkpoint page {:#x} is not 4KiB aligned", address.get()),
            ));
        }
    }
    if canonical
        .windows(2)
        .any(|pair| pair[0].get() == pair[1].get())
    {
        return Err(page_set_error(
            "bounded page-set checkpoint capture",
            "checkpoint ownership set contains a duplicate page",
        ));
    }
    Ok(canonical)
}

fn page_set_error(operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_set_is_nonempty_bounded_aligned_unique_and_canonical() {
        assert!(validate_page_addresses(&[]).is_err());
        assert!(validate_page_addresses(&[GuestPhysAddr::new(0x30001)]).is_err());
        assert!(validate_page_addresses(&[
            GuestPhysAddr::new(0x30000),
            GuestPhysAddr::new(0x30000),
        ])
        .is_err());

        let too_many = (0..=BOUNDED_CHECKPOINT_PAGE_SET_LIMIT)
            .map(|index| GuestPhysAddr::new((index as u64 + 1) * LONG_MODE_PAGE_SIZE))
            .collect::<Vec<_>>();
        assert!(validate_page_addresses(&too_many).is_err());

        assert_eq!(
            validate_page_addresses(&[
                GuestPhysAddr::new(0x1fe000),
                GuestPhysAddr::new(0x30000),
                GuestPhysAddr::new(0x31000),
            ])
            .unwrap(),
            [
                GuestPhysAddr::new(0x30000),
                GuestPhysAddr::new(0x31000),
                GuestPhysAddr::new(0x1fe000),
            ]
        );
    }
}

include!("page_set/controller.rs");
include!("page_set/full_controller.rs");
