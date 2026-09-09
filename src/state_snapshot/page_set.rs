use super::{VcpuStateSnapshot, VcpuStateSnapshotComparison};
use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::msr::GuestMsrAccessPolicy;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{
    LongModeBootLayout, LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE,
    LONG_MODE_PAGE_SIZE_USIZE,
};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::{PortIoBus, DEBUG_PORT};
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

pub const BOUNDED_CHECKPOINT_PAGE_SET_LIMIT: usize = 8;

pub const MULTI_PAGE_CHECKPOINT_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x10000);
pub const MULTI_PAGE_CHECKPOINT_CONTROL_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x30000);
pub const MULTI_PAGE_CHECKPOINT_DATA_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x31000);
pub const MULTI_PAGE_CHECKPOINT_STACK_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x1fe000);
pub const MULTI_PAGE_CHECKPOINT_STACK_POINTER: u64 = 0x1feff8;
pub const MULTI_PAGE_CHECKPOINT_STACK_VALUE_ADDR: GuestPhysAddr = GuestPhysAddr::new(0x1feff0);
pub const MULTI_PAGE_CHECKPOINT_CORRUPT_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x12000);
pub const MULTI_PAGE_CHECKPOINT_CORRUPT_STACK: u64 = 0x1fdff8;
pub const MULTI_PAGE_CHECKPOINT_CAPTURE_RIP: u64 = 0x10013;
pub const MULTI_PAGE_CHECKPOINT_TERMINAL_RIP: u64 = 0x1002d;
pub const MULTI_PAGE_CHECKPOINT_PROOF: &[u8; 4] = b"ABCR";
pub const MULTI_PAGE_CHECKPOINT_CONTROL_MARKER: u8 = b'A';
pub const MULTI_PAGE_CHECKPOINT_DATA_MARKER: u8 = b'B';
pub const MULTI_PAGE_CHECKPOINT_STACK_MARKER: u8 = b'C';
pub const MULTI_PAGE_CHECKPOINT_OWNERSHIP_SET: [GuestPhysAddr; 3] = [
    MULTI_PAGE_CHECKPOINT_CONTROL_PAGE,
    MULTI_PAGE_CHECKPOINT_DATA_PAGE,
    MULTI_PAGE_CHECKPOINT_STACK_PAGE,
];

const MULTI_PAGE_CHECKPOINT_GUEST_BYTES: [u8; 45] = [
    0xc6, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00, MULTI_PAGE_CHECKPOINT_CONTROL_MARKER,
    0xc6, 0x04, 0x25, 0x00, 0x10, 0x03, 0x00, MULTI_PAGE_CHECKPOINT_DATA_MARKER,
    0x6a, MULTI_PAGE_CHECKPOINT_STACK_MARKER, // push 0x43 onto the owned stack page
    0xf4, // checkpoint HLT
    0x8a, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00, // mov al, [0x30000]
    0xe6, 0xe9, // out 0xe9, al => A
    0x8a, 0x04, 0x25, 0x00, 0x10, 0x03, 0x00, // mov al, [0x31000]
    0xe6, 0xe9, // out 0xe9, al => B
    0x58, // pop rax => C from the restored stack page
    0xe6, 0xe9, // out 0xe9, al => C
    0xb0, b'R', // mov al, 'R'
    0xe6, 0xe9, // out 0xe9, al => R
    0xf4, // terminal HLT
];

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
            let mut bytes = vec![0_u8; LONG_MODE_PAGE_SIZE_USIZE];
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
        page_exact_in(&self.pages, address)
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.all_pages_exact() && self.vcpu.is_exact_match()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiPageCheckpointGuestResult {
    checkpoint_report: VmExitReport,
    captured_pages: Vec<GuestPhysAddr>,
    corruption: BoundedPageSetCheckpointComparison,
    restored: BoundedPageSetCheckpointComparison,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    terminal_report: VmExitReport,
}

impl MultiPageCheckpointGuestResult {
    #[must_use]
    pub const fn checkpoint_report(&self) -> VmExitReport {
        self.checkpoint_report
    }

    #[must_use]
    pub fn captured_pages(&self) -> &[GuestPhysAddr] {
        &self.captured_pages
    }

    #[must_use]
    pub const fn corruption(&self) -> &BoundedPageSetCheckpointComparison {
        &self.corruption
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedPageSetCheckpointComparison {
        &self.restored
    }

    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn terminal_report(&self) -> VmExitReport {
        self.terminal_report
    }
}

pub fn run_multi_page_checkpoint_guest(
    config: VmConfig,
) -> Result<MultiPageCheckpointGuestResult, Error> {
    let image = FlatGuestImage::new(
        MULTI_PAGE_CHECKPOINT_ENTRY,
        MULTI_PAGE_CHECKPOINT_ENTRY,
        &MULTI_PAGE_CHECKPOINT_GUEST_BYTES,
    )?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeBootLayout::new(
        memory.region(),
        image.entry(),
        MULTI_PAGE_CHECKPOINT_STACK_POINTER,
    )
    .expect("fixed multi-page checkpoint layout remains valid");
    let corrupt_layout = LongModeBootLayout::new(
        memory.region(),
        MULTI_PAGE_CHECKPOINT_CORRUPT_ENTRY,
        MULTI_PAGE_CHECKPOINT_CORRUPT_STACK,
    )
    .expect("fixed multi-page checkpoint corruption layout remains valid");
    layout.install_page_tables(&mut memory)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode(&layout)?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty multi-page checkpoint MSR policy is valid by construction");

    let mut no_io = PortIoBus::empty();
    let checkpoint_execution = run_vcpu_until_stopped(&mut vcpu, &mut no_io, 1)?;
    let checkpoint_report = checkpoint_execution.report();
    require_hlt_report(
        "multi-page checkpoint capture boundary",
        checkpoint_report,
        MULTI_PAGE_CHECKPOINT_CAPTURE_RIP,
    )?;
    if !checkpoint_execution.io_exits().is_empty() {
        return Err(page_set_error(
            "multi-page checkpoint capture boundary",
            "capture boundary unexpectedly serviced port I/O",
        ));
    }

    let checkpoint = BoundedVcpuPageSetCheckpoint::capture(
        &vcpu,
        &msr_policy,
        vm.guest_memory()
            .expect("registered multi-page checkpoint memory remains VM-owned"),
        &MULTI_PAGE_CHECKPOINT_OWNERSHIP_SET,
    )?;
    require_captured_roles(&checkpoint)?;

    let captured_pages = checkpoint
        .pages()
        .iter()
        .map(BoundedCheckpointPage::address)
        .collect::<Vec<_>>();

    corrupt_owned_pages(
        vm.guest_memory_mut()
            .expect("registered multi-page checkpoint memory remains VM-owned"),
    )?;
    vcpu.initialize_long_mode(&corrupt_layout)?;

    let corruption = checkpoint.verify(
        &vcpu,
        vm.guest_memory()
            .expect("registered multi-page checkpoint memory remains VM-owned"),
    )?;
    require_all_owned_pages_mismatched("multi-page checkpoint corruption proof", &corruption)?;
    if corruption.vcpu().is_exact_match() {
        return Err(page_set_error(
            "multi-page checkpoint corruption proof",
            "intentional VCPU corruption remained an exact match",
        ));
    }

    let restored = checkpoint.restore_and_verify(
        &vcpu,
        vm.guest_memory_mut()
            .expect("registered multi-page checkpoint memory remains VM-owned"),
    )?;
    if !restored.is_exact_match() {
        return Err(page_set_error(
            "multi-page checkpoint restore verification",
            format!(
                "restore mismatch: pages={:?} vcpu_exact={}",
                restored.pages(),
                restored.vcpu().is_exact_match()
            ),
        ));
    }

    let mut port_io = PortIoBus::with_debug_port();
    let resumed = run_vcpu_until_stopped(&mut vcpu, &mut port_io, 5)?;
    let terminal_report = resumed.report();
    require_hlt_report(
        "multi-page checkpoint resumed terminal",
        terminal_report,
        MULTI_PAGE_CHECKPOINT_TERMINAL_RIP,
    )?;
    if resumed.io_exits().len() != MULTI_PAGE_CHECKPOINT_PROOF.len() {
        return Err(page_set_error(
            "multi-page checkpoint resumed proof",
            format!(
                "expected {} debug-port exits, got {}",
                MULTI_PAGE_CHECKPOINT_PROOF.len(),
                resumed.io_exits().len()
            ),
        ));
    }
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != MULTI_PAGE_CHECKPOINT_PROOF {
        return Err(page_set_error(
            "multi-page checkpoint resumed proof",
            format!("expected {:?}, got {proof:?}", MULTI_PAGE_CHECKPOINT_PROOF),
        ));
    }
    for (io, expected) in resumed
        .io_exits()
        .iter()
        .zip(MULTI_PAGE_CHECKPOINT_PROOF.iter().copied())
    {
        if io.direction() != PortIoDirection::Out
            || io.port() != DEBUG_PORT
            || io.size() != 1
            || io.count() != 1
            || io.output_data() != [expected]
        {
            return Err(page_set_error(
                "multi-page checkpoint resumed I/O",
                format!("unexpected debug-port exit {io:?}"),
            ));
        }
    }

    Ok(MultiPageCheckpointGuestResult {
        checkpoint_report,
        captured_pages,
        corruption,
        restored,
        io_exits: resumed.io_exits().to_vec(),
        proof,
        terminal_report,
    })
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

fn page_exact_in(
    pages: &[BoundedCheckpointPageComparison],
    address: GuestPhysAddr,
) -> Option<bool> {
    pages
        .binary_search_by_key(&address.get(), |page| page.address.get())
        .ok()
        .map(|index| pages[index].exact)
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

fn require_captured_roles(checkpoint: &BoundedVcpuPageSetCheckpoint) -> Result<(), Error> {
    let control = checkpoint
        .page(MULTI_PAGE_CHECKPOINT_CONTROL_PAGE)
        .and_then(|page| page.bytes().first())
        .copied();
    let data = checkpoint
        .page(MULTI_PAGE_CHECKPOINT_DATA_PAGE)
        .and_then(|page| page.bytes().first())
        .copied();
    let stack_offset = usize::try_from(
        MULTI_PAGE_CHECKPOINT_STACK_VALUE_ADDR.get() - MULTI_PAGE_CHECKPOINT_STACK_PAGE.get(),
    )
    .expect("fixed stack marker offset fits host usize");
    let stack = checkpoint
        .page(MULTI_PAGE_CHECKPOINT_STACK_PAGE)
        .and_then(|page| page.bytes().get(stack_offset))
        .copied();
    if control != Some(MULTI_PAGE_CHECKPOINT_CONTROL_MARKER)
        || data != Some(MULTI_PAGE_CHECKPOINT_DATA_MARKER)
        || stack != Some(MULTI_PAGE_CHECKPOINT_STACK_MARKER)
    {
        return Err(page_set_error(
            "multi-page checkpoint role capture",
            format!("unexpected captured roles: control={control:?} data={data:?} stack={stack:?}"),
        ));
    }
    Ok(())
}

fn corrupt_owned_pages(memory: &mut GuestMemory) -> Result<(), Error> {
    memory.write(
        MULTI_PAGE_CHECKPOINT_CONTROL_PAGE,
        &vec![0xa5; LONG_MODE_PAGE_SIZE_USIZE],
    )?;
    memory.write(
        MULTI_PAGE_CHECKPOINT_DATA_PAGE,
        &vec![0x5a; LONG_MODE_PAGE_SIZE_USIZE],
    )?;
    memory.write(
        MULTI_PAGE_CHECKPOINT_STACK_PAGE,
        &vec![0xcc; LONG_MODE_PAGE_SIZE_USIZE],
    )?;
    Ok(())
}

fn require_all_owned_pages_mismatched(
    operation: &'static str,
    comparison: &BoundedPageSetCheckpointComparison,
) -> Result<(), Error> {
    for address in MULTI_PAGE_CHECKPOINT_OWNERSHIP_SET {
        if comparison.page_exact(address) != Some(false) {
            return Err(page_set_error(
                operation,
                format!(
                    "owned page {:#x} did not independently prove mismatch: {:?}",
                    address.get(),
                    comparison.pages()
                ),
            ));
        }
    }
    Ok(())
}

fn require_hlt_report(
    operation: &'static str,
    report: VmExitReport,
    expected_rip: u64,
) -> Result<(), Error> {
    if report.exit() != VcpuExit::Hlt
        || report.rip() != expected_rip
        || report.rflags() & 0x2 != 0x2
    {
        return Err(page_set_error(
            operation,
            format!(
                "expected HLT at rip={expected_rip:#x} with architectural RFLAGS bit1, got {report}"
            ),
        ));
    }
    Ok(())
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
                MULTI_PAGE_CHECKPOINT_STACK_PAGE,
                MULTI_PAGE_CHECKPOINT_CONTROL_PAGE,
                MULTI_PAGE_CHECKPOINT_DATA_PAGE,
            ])
            .unwrap(),
            [
                MULTI_PAGE_CHECKPOINT_CONTROL_PAGE,
                MULTI_PAGE_CHECKPOINT_DATA_PAGE,
                MULTI_PAGE_CHECKPOINT_STACK_PAGE,
            ]
        );
    }

    #[test]
    fn page_lookup_uses_canonical_addresses_without_vcpu_fixture() {
        let pages = vec![
            BoundedCheckpointPageComparison {
                address: GuestPhysAddr::new(0x22000),
                exact: true,
            },
            BoundedCheckpointPageComparison {
                address: GuestPhysAddr::new(0x30000),
                exact: false,
            },
        ];
        assert_eq!(page_exact_in(&pages, GuestPhysAddr::new(0x22000)), Some(true));
        assert_eq!(page_exact_in(&pages, GuestPhysAddr::new(0x30000)), Some(false));
        assert_eq!(page_exact_in(&pages, GuestPhysAddr::new(0x40000)), None);
    }

    #[test]
    fn deterministic_multi_page_guest_uses_three_roles_and_two_hlt_boundaries() {
        assert_eq!(MULTI_PAGE_CHECKPOINT_GUEST_BYTES.len(), 45);
        assert_eq!(MULTI_PAGE_CHECKPOINT_GUEST_BYTES[18], 0xf4);
        assert_eq!(MULTI_PAGE_CHECKPOINT_GUEST_BYTES[44], 0xf4);
        assert_eq!(
            MULTI_PAGE_CHECKPOINT_CAPTURE_RIP,
            MULTI_PAGE_CHECKPOINT_ENTRY.get() + 19
        );
        assert_eq!(
            MULTI_PAGE_CHECKPOINT_TERMINAL_RIP,
            MULTI_PAGE_CHECKPOINT_ENTRY.get() + 45
        );
        assert_eq!(MULTI_PAGE_CHECKPOINT_STACK_POINTER - 8, MULTI_PAGE_CHECKPOINT_STACK_VALUE_ADDR.get());
    }
}
