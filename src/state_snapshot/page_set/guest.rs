use super::{
    page_set_error, BoundedCheckpointPage, BoundedPageSetCheckpointComparison,
    BoundedVcpuPageSetCheckpoint,
};
use crate::config::VmConfig;
use crate::error::Error;
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::msr::GuestMsrAccessPolicy;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LongModeBootLayout, LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::{PortIoBus, DEBUG_PORT};
use crate::vcpu::{PortIoDirection, PortIoExit, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;

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
    0xc6,
    0x04,
    0x25,
    0x00,
    0x00,
    0x03,
    0x00,
    MULTI_PAGE_CHECKPOINT_CONTROL_MARKER,
    0xc6,
    0x04,
    0x25,
    0x00,
    0x10,
    0x03,
    0x00,
    MULTI_PAGE_CHECKPOINT_DATA_MARKER,
    0x6a,
    MULTI_PAGE_CHECKPOINT_STACK_MARKER, // push 0x43 onto the owned stack page
    0xf4,                               // checkpoint HLT
    0x8a,
    0x04,
    0x25,
    0x00,
    0x00,
    0x03,
    0x00, // mov al, [0x30000]
    0xe6,
    0xe9, // out 0xe9, al => A
    0x8a,
    0x04,
    0x25,
    0x00,
    0x10,
    0x03,
    0x00, // mov al, [0x31000]
    0xe6,
    0xe9, // out 0xe9, al => B
    0x58, // pop rax => C from the restored stack page
    0xe6,
    0xe9, // out 0xe9, al => C
    0xb0,
    b'R', // mov al, 'R'
    0xe6,
    0xe9, // out 0xe9, al => R
    0xf4, // terminal HLT
];

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
        &vec![0xa5; LONG_MODE_PAGE_SIZE as usize],
    )?;
    memory.write(
        MULTI_PAGE_CHECKPOINT_DATA_PAGE,
        &vec![0x5a; LONG_MODE_PAGE_SIZE as usize],
    )?;
    memory.write(
        MULTI_PAGE_CHECKPOINT_STACK_PAGE,
        &vec![0xcc; LONG_MODE_PAGE_SIZE as usize],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_guest_binds_three_page_roles_to_resume_behavior() {
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
        assert_eq!(
            MULTI_PAGE_CHECKPOINT_STACK_POINTER - 8,
            MULTI_PAGE_CHECKPOINT_STACK_VALUE_ADDR.get()
        );
    }
}
