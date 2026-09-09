use super::{
    BoundedCheckpointPage, BoundedPageSetCheckpointComparison, BoundedVcpuPageSetCheckpoint,
    VcpuStateSnapshot, VcpuStateSnapshotComparison,
};
use crate::error::{Error, HostEnvironmentError};
use crate::execution::run_vcpu_until_stopped;
use crate::kvm::msr::GuestMsrAccessPolicy;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LongModeBootLayout, LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::{PortIoBus, DEBUG_PORT};
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;
use std::io;

pub const TWO_VCPU_CHECKPOINT_FIRST_ID: VcpuId = VcpuId::BOOT;
pub const TWO_VCPU_CHECKPOINT_SECOND_ID: VcpuId = VcpuId::new(1);
pub const TWO_VCPU_CHECKPOINT_FIRST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x10000);
pub const TWO_VCPU_CHECKPOINT_SECOND_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x11000);
pub const TWO_VCPU_CHECKPOINT_SHARED_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x30000);
pub const TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x1fd000);
pub const TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x1fc000);
pub const TWO_VCPU_CHECKPOINT_FIRST_STACK: u64 = 0x1fdff8;
pub const TWO_VCPU_CHECKPOINT_SECOND_STACK: u64 = 0x1fcff8;
pub const TWO_VCPU_CHECKPOINT_SHARED_MARKER: u8 = b'S';
pub const TWO_VCPU_CHECKPOINT_FIRST_MARKER: u8 = b'0';
pub const TWO_VCPU_CHECKPOINT_SECOND_MARKER: u8 = b'1';
pub const TWO_VCPU_CHECKPOINT_FIRST_PROOF: &[u8; 1] = b"0";
pub const TWO_VCPU_CHECKPOINT_SECOND_PROOF: &[u8; 1] = b"1";
pub const TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP: u64 = 0x1000b;
pub const TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP: u64 = 0x11003;
pub const TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP: u64 = 0x10020;
pub const TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP: u64 = 0x11018;
pub const TWO_VCPU_CHECKPOINT_OWNERSHIP_SET: [GuestPhysAddr; 3] = [
    TWO_VCPU_CHECKPOINT_SHARED_PAGE,
    TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
    TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE,
];

const FIRST_GUEST_BYTES: [u8; 37] = [
    0xc6,
    0x04,
    0x25,
    0x00,
    0x00,
    0x03,
    0x00,
    TWO_VCPU_CHECKPOINT_SHARED_MARKER, // mov byte [0x30000], 'S'
    0x6a,
    TWO_VCPU_CHECKPOINT_FIRST_MARKER, // push '0'
    0xf4,                             // coordinated capture HLT
    0x58,                             // pop rax => restored '0'
    0x3c,
    TWO_VCPU_CHECKPOINT_FIRST_MARKER, // cmp al, '0'
    0x75,
    0x10, // jne failure
    0x8a,
    0x04,
    0x25,
    0x00,
    0x00,
    0x03,
    0x00, // mov al, [0x30000]
    0x3c,
    TWO_VCPU_CHECKPOINT_SHARED_MARKER,
    0x75,
    0x05, // jne failure
    0xb0,
    TWO_VCPU_CHECKPOINT_FIRST_MARKER,
    0xe6,
    0xe9, // out 0xe9, al => 0
    0xf4, // success terminal HLT
    0xb0,
    b'F',
    0xe6,
    0xe9,
    0xf4,
];

const SECOND_GUEST_BYTES: [u8; 29] = [
    0x6a,
    TWO_VCPU_CHECKPOINT_SECOND_MARKER, // push '1'
    0xf4,                              // coordinated capture HLT
    0x58,                              // pop rax => restored '1'
    0x3c,
    TWO_VCPU_CHECKPOINT_SECOND_MARKER,
    0x75,
    0x10, // jne failure
    0x8a,
    0x04,
    0x25,
    0x00,
    0x00,
    0x03,
    0x00, // mov al, [0x30000]
    0x3c,
    TWO_VCPU_CHECKPOINT_SHARED_MARKER,
    0x75,
    0x05, // jne failure
    0xb0,
    TWO_VCPU_CHECKPOINT_SECOND_MARKER,
    0xe6,
    0xe9, // out 0xe9, al => 1
    0xf4, // success terminal HLT
    0xb0,
    b'F',
    0xe6,
    0xe9,
    0xf4,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedTwoVcpuCheckpoint {
    primary_id: VcpuId,
    primary: BoundedVcpuPageSetCheckpoint,
    secondary_id: VcpuId,
    secondary: VcpuStateSnapshot,
}

impl BoundedTwoVcpuCheckpoint {
    pub fn capture(
        first: &Vcpu,
        second: &Vcpu,
        msr_policy: &GuestMsrAccessPolicy,
        memory: &GuestMemory,
        page_addresses: &[GuestPhysAddr],
    ) -> Result<Self, Error> {
        let (primary, secondary) = canonical_vcpu_pair(first, second)?;
        let primary =
            BoundedVcpuPageSetCheckpoint::capture(primary, msr_policy, memory, page_addresses)?;
        let secondary_snapshot = secondary.capture_state_snapshot(msr_policy)?;
        Ok(Self {
            primary_id: primary.vcpu().registers().id(),
            primary,
            secondary_id: secondary.id(),
            secondary: secondary_snapshot,
        })
    }

    #[must_use]
    pub const fn vcpu_ids(&self) -> [VcpuId; 2] {
        [self.primary_id, self.secondary_id]
    }

    #[must_use]
    pub fn pages(&self) -> &[BoundedCheckpointPage] {
        self.primary.pages()
    }

    pub fn verify(
        &self,
        first: &Vcpu,
        second: &Vcpu,
        memory: &GuestMemory,
    ) -> Result<BoundedTwoVcpuCheckpointComparison, Error> {
        let (primary, secondary) = self.bind_vcpus(first, second)?;
        Ok(BoundedTwoVcpuCheckpointComparison {
            primary_id: self.primary_id,
            primary: self.primary.verify(primary, memory)?,
            secondary_id: self.secondary_id,
            secondary: secondary.verify_state_snapshot(&self.secondary)?,
        })
    }

    pub fn restore_and_verify(
        &self,
        first: &Vcpu,
        second: &Vcpu,
        memory: &mut GuestMemory,
    ) -> Result<BoundedTwoVcpuCheckpointComparison, Error> {
        let (primary, secondary) = self.bind_vcpus(first, second)?;
        Ok(BoundedTwoVcpuCheckpointComparison {
            primary_id: self.primary_id,
            primary: self.primary.restore_and_verify(primary, memory)?,
            secondary_id: self.secondary_id,
            secondary: secondary.restore_and_verify_state_snapshot(&self.secondary)?,
        })
    }

    fn bind_vcpus<'a>(
        &self,
        first: &'a Vcpu,
        second: &'a Vcpu,
    ) -> Result<(&'a Vcpu, &'a Vcpu), Error> {
        let (primary, secondary) = canonical_vcpu_pair(first, second)?;
        if [primary.id(), secondary.id()] != self.vcpu_ids() {
            return Err(two_vcpu_checkpoint_error(
                primary.id(),
                "two-vCPU checkpoint binding",
                format!(
                    "checkpoint owns {:?}, supplied {:?}",
                    self.vcpu_ids(),
                    [primary.id(), secondary.id()]
                ),
            ));
        }
        Ok((primary, secondary))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedTwoVcpuCheckpointComparison {
    primary_id: VcpuId,
    primary: BoundedPageSetCheckpointComparison,
    secondary_id: VcpuId,
    secondary: VcpuStateSnapshotComparison,
}

impl BoundedTwoVcpuCheckpointComparison {
    #[must_use]
    pub fn pages(&self) -> &[super::BoundedCheckpointPageComparison] {
        self.primary.pages()
    }

    #[must_use]
    pub fn page_exact(&self, address: GuestPhysAddr) -> Option<bool> {
        self.primary.page_exact(address)
    }

    #[must_use]
    pub fn vcpu_exact(&self, id: VcpuId) -> Option<bool> {
        if id == self.primary_id {
            Some(self.primary.vcpu().is_exact_match())
        } else if id == self.secondary_id {
            Some(self.secondary.is_exact_match())
        } else {
            None
        }
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.primary.is_exact_match() && self.secondary.is_exact_match()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuCheckpointGuestResult {
    first_capture: VmExitReport,
    second_capture: VmExitReport,
    captured_pages: Vec<GuestPhysAddr>,
    corruption: BoundedTwoVcpuCheckpointComparison,
    restored: BoundedTwoVcpuCheckpointComparison,
    first_io_exits: Vec<PortIoExit>,
    second_io_exits: Vec<PortIoExit>,
    first_proof: Vec<u8>,
    second_proof: Vec<u8>,
    first_terminal: VmExitReport,
    second_terminal: VmExitReport,
}

impl TwoVcpuCheckpointGuestResult {
    #[must_use]
    pub const fn first_capture(&self) -> VmExitReport {
        self.first_capture
    }
    #[must_use]
    pub const fn second_capture(&self) -> VmExitReport {
        self.second_capture
    }
    #[must_use]
    pub fn captured_pages(&self) -> &[GuestPhysAddr] {
        &self.captured_pages
    }
    #[must_use]
    pub const fn corruption(&self) -> &BoundedTwoVcpuCheckpointComparison {
        &self.corruption
    }
    #[must_use]
    pub const fn restored(&self) -> &BoundedTwoVcpuCheckpointComparison {
        &self.restored
    }
    #[must_use]
    pub fn first_io_exits(&self) -> &[PortIoExit] {
        &self.first_io_exits
    }
    #[must_use]
    pub fn second_io_exits(&self) -> &[PortIoExit] {
        &self.second_io_exits
    }
    #[must_use]
    pub fn first_proof(&self) -> &[u8] {
        &self.first_proof
    }
    #[must_use]
    pub fn second_proof(&self) -> &[u8] {
        &self.second_proof
    }
    #[must_use]
    pub const fn first_terminal(&self) -> VmExitReport {
        self.first_terminal
    }
    #[must_use]
    pub const fn second_terminal(&self) -> VmExitReport {
        self.second_terminal
    }
}

pub fn run_two_vcpu_checkpoint_guest() -> Result<TwoVcpuCheckpointGuestResult, Error> {
    let first_image = FlatGuestImage::new(
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        &FIRST_GUEST_BYTES,
    )?;
    let second_image = FlatGuestImage::new(
        TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
        TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
        &SECOND_GUEST_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let first_layout = LongModeBootLayout::new(
        memory.region(),
        first_image.entry(),
        TWO_VCPU_CHECKPOINT_FIRST_STACK,
    )
    .expect("fixed first two-vCPU checkpoint layout remains valid");
    let second_layout = LongModeBootLayout::new(
        memory.region(),
        second_image.entry(),
        TWO_VCPU_CHECKPOINT_SECOND_STACK,
    )
    .expect("fixed second two-vCPU checkpoint layout remains valid");
    let first_corrupt_layout = LongModeBootLayout::new(
        memory.region(),
        GuestPhysAddr::new(0x12000),
        0x1fbff8,
    )
    .expect("fixed first corruption layout remains valid");
    let second_corrupt_layout = LongModeBootLayout::new(
        memory.region(),
        GuestPhysAddr::new(0x13000),
        0x1faff8,
    )
    .expect("fixed second corruption layout remains valid");
    first_layout.install_page_tables(&mut memory)?;
    first_image.load(&mut memory)?;
    second_image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    let mut first_vcpu = vm.create_vcpu(TWO_VCPU_CHECKPOINT_FIRST_ID)?;
    let mut second_vcpu = vm.create_vcpu(TWO_VCPU_CHECKPOINT_SECOND_ID)?;
    first_vcpu.initialize_long_mode(&first_layout)?;
    second_vcpu.initialize_long_mode(&second_layout)?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty two-vCPU checkpoint MSR policy is valid by construction");

    let first_capture = run_to_quiescent_hlt(
        &mut first_vcpu,
        TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP,
        "first two-vCPU checkpoint quiescence",
    )?;
    let second_capture = run_to_quiescent_hlt(
        &mut second_vcpu,
        TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP,
        "second two-vCPU checkpoint quiescence",
    )?;

    let checkpoint = BoundedTwoVcpuCheckpoint::capture(
        &first_vcpu,
        &second_vcpu,
        &msr_policy,
        vm.guest_memory()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
        &TWO_VCPU_CHECKPOINT_OWNERSHIP_SET,
    )?;
    require_captured_roles(&checkpoint)?;
    let captured_pages = checkpoint
        .pages()
        .iter()
        .map(BoundedCheckpointPage::address)
        .collect::<Vec<_>>();

    corrupt_owned_pages(
        vm.guest_memory_mut()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
    )?;
    first_vcpu.initialize_long_mode(&first_corrupt_layout)?;
    second_vcpu.initialize_long_mode(&second_corrupt_layout)?;

    let corruption = checkpoint.verify(
        &first_vcpu,
        &second_vcpu,
        vm.guest_memory()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
    )?;
    require_full_mismatch("two-vCPU checkpoint corruption proof", &corruption)?;

    let restored = checkpoint.restore_and_verify(
        &first_vcpu,
        &second_vcpu,
        vm.guest_memory_mut()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
    )?;
    if !restored.is_exact_match() {
        return Err(two_vcpu_checkpoint_error(
            TWO_VCPU_CHECKPOINT_FIRST_ID,
            "two-vCPU checkpoint restore verification",
            format!(
                "restore mismatch: pages={:?}, vcpu0={:?}, vcpu1={:?}",
                restored.pages(),
                restored.vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                restored.vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID)
            ),
        ));
    }

    let (first_io_exits, first_proof, first_terminal) = resume_and_verify(
        &mut first_vcpu,
        TWO_VCPU_CHECKPOINT_FIRST_PROOF,
        TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP,
        "first two-vCPU checkpoint resume",
    )?;
    let (second_io_exits, second_proof, second_terminal) = resume_and_verify(
        &mut second_vcpu,
        TWO_VCPU_CHECKPOINT_SECOND_PROOF,
        TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP,
        "second two-vCPU checkpoint resume",
    )?;

    Ok(TwoVcpuCheckpointGuestResult {
        first_capture,
        second_capture,
        captured_pages,
        corruption,
        restored,
        first_io_exits,
        second_io_exits,
        first_proof,
        second_proof,
        first_terminal,
        second_terminal,
    })
}

fn canonical_vcpu_pair<'a>(
    first: &'a Vcpu,
    second: &'a Vcpu,
) -> Result<(&'a Vcpu, &'a Vcpu), Error> {
    if first.id() == second.id() {
        return Err(two_vcpu_checkpoint_error(
            first.id(),
            "two-vCPU checkpoint ownership",
            "checkpoint requires two distinct vCPU ids",
        ));
    }
    if first.id().get() < second.id().get() {
        Ok((first, second))
    } else {
        Ok((second, first))
    }
}

fn run_to_quiescent_hlt(
    vcpu: &mut Vcpu,
    expected_rip: u64,
    operation: &'static str,
) -> Result<VmExitReport, Error> {
    let mut no_io = PortIoBus::empty();
    let execution = run_vcpu_until_stopped(vcpu, &mut no_io, 1)?;
    if !execution.io_exits().is_empty() {
        return Err(two_vcpu_checkpoint_error(
            vcpu.id(),
            operation,
            "quiescence boundary unexpectedly serviced port I/O",
        ));
    }
    require_hlt_report(vcpu.id(), operation, execution.report(), expected_rip)?;
    Ok(execution.report())
}

fn require_captured_roles(checkpoint: &BoundedTwoVcpuCheckpoint) -> Result<(), Error> {
    let page = |address| checkpoint.pages().iter().find(|page| page.address() == address);
    let shared = page(TWO_VCPU_CHECKPOINT_SHARED_PAGE)
        .and_then(|page| page.bytes().first())
        .copied();
    let first_offset = usize::try_from(
        TWO_VCPU_CHECKPOINT_FIRST_STACK - 8 - TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE.get(),
    )
    .expect("fixed first stack marker offset fits usize");
    let second_offset = usize::try_from(
        TWO_VCPU_CHECKPOINT_SECOND_STACK - 8 - TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE.get(),
    )
    .expect("fixed second stack marker offset fits usize");
    let first_stack = page(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE)
        .and_then(|page| page.bytes().get(first_offset))
        .copied();
    let second_stack = page(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE)
        .and_then(|page| page.bytes().get(second_offset))
        .copied();
    if shared != Some(TWO_VCPU_CHECKPOINT_SHARED_MARKER)
        || first_stack != Some(TWO_VCPU_CHECKPOINT_FIRST_MARKER)
        || second_stack != Some(TWO_VCPU_CHECKPOINT_SECOND_MARKER)
    {
        return Err(two_vcpu_checkpoint_error(
            TWO_VCPU_CHECKPOINT_FIRST_ID,
            "two-vCPU checkpoint role capture",
            format!(
                "unexpected roles: shared={shared:?}, first_stack={first_stack:?}, second_stack={second_stack:?}"
            ),
        ));
    }
    Ok(())
}

fn corrupt_owned_pages(memory: &mut GuestMemory) -> Result<(), Error> {
    memory.write(
        TWO_VCPU_CHECKPOINT_SHARED_PAGE,
        &vec![0xa5; LONG_MODE_PAGE_SIZE as usize],
    )?;
    memory.write(
        TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE,
        &vec![0x5a; LONG_MODE_PAGE_SIZE as usize],
    )?;
    memory.write(
        TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
        &vec![0xcc; LONG_MODE_PAGE_SIZE as usize],
    )?;
    Ok(())
}

fn require_full_mismatch(
    operation: &'static str,
    comparison: &BoundedTwoVcpuCheckpointComparison,
) -> Result<(), Error> {
    for address in TWO_VCPU_CHECKPOINT_OWNERSHIP_SET {
        if comparison.page_exact(address) != Some(false) {
            return Err(two_vcpu_checkpoint_error(
                TWO_VCPU_CHECKPOINT_FIRST_ID,
                operation,
                format!("owned page {:#x} did not independently mismatch", address.get()),
            ));
        }
    }
    for id in [TWO_VCPU_CHECKPOINT_FIRST_ID, TWO_VCPU_CHECKPOINT_SECOND_ID] {
        if comparison.vcpu_exact(id) != Some(false) {
            return Err(two_vcpu_checkpoint_error(
                id,
                operation,
                "intentional vCPU corruption remained exact",
            ));
        }
    }
    Ok(())
}

fn resume_and_verify(
    vcpu: &mut Vcpu,
    expected_proof: &[u8],
    expected_rip: u64,
    operation: &'static str,
) -> Result<(Vec<PortIoExit>, Vec<u8>, VmExitReport), Error> {
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(vcpu, &mut port_io, 2)?;
    require_hlt_report(vcpu.id(), operation, execution.report(), expected_rip)?;
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != expected_proof || execution.io_exits().len() != expected_proof.len() {
        return Err(two_vcpu_checkpoint_error(
            vcpu.id(),
            operation,
            format!("expected proof {expected_proof:?}, got {proof:?}"),
        ));
    }
    for (io, expected) in execution.io_exits().iter().zip(expected_proof.iter().copied()) {
        if io.direction() != PortIoDirection::Out
            || io.port() != DEBUG_PORT
            || io.size() != 1
            || io.count() != 1
            || io.output_data() != [expected]
        {
            return Err(two_vcpu_checkpoint_error(
                vcpu.id(),
                operation,
                format!("unexpected debug-port exit {io:?}"),
            ));
        }
    }
    Ok((execution.io_exits().to_vec(), proof, execution.report()))
}

fn require_hlt_report(
    id: VcpuId,
    operation: &'static str,
    report: VmExitReport,
    expected_rip: u64,
) -> Result<(), Error> {
    if report.exit() != VcpuExit::Hlt
        || report.vcpu_id() != id
        || report.rip() != expected_rip
        || report.rflags() & 0x2 != 0x2
    {
        return Err(two_vcpu_checkpoint_error(
            id,
            operation,
            format!(
                "expected vCPU {} HLT at rip={expected_rip:#x}, got {report}",
                id.get()
            ),
        ));
    }
    Ok(())
}

fn two_vcpu_checkpoint_error(
    id: VcpuId,
    operation: &'static str,
    detail: impl Into<String>,
) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: id.get(),
        operation,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_guests_have_distinct_quiescent_and_terminal_boundaries() {
        assert_eq!(FIRST_GUEST_BYTES.len(), 37);
        assert_eq!(SECOND_GUEST_BYTES.len(), 29);
        assert_eq!(FIRST_GUEST_BYTES[10], 0xf4);
        assert_eq!(SECOND_GUEST_BYTES[2], 0xf4);
        assert_eq!(FIRST_GUEST_BYTES[31], 0xf4);
        assert_eq!(SECOND_GUEST_BYTES[23], 0xf4);
        assert_eq!(
            TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP,
            TWO_VCPU_CHECKPOINT_FIRST_ENTRY.get() + 11
        );
        assert_eq!(
            TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP,
            TWO_VCPU_CHECKPOINT_SECOND_ENTRY.get() + 3
        );
        assert_eq!(
            TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP,
            TWO_VCPU_CHECKPOINT_FIRST_ENTRY.get() + 32
        );
        assert_eq!(
            TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP,
            TWO_VCPU_CHECKPOINT_SECOND_ENTRY.get() + 24
        );
    }

    #[test]
    fn ownership_set_contains_shared_and_both_active_stack_pages() {
        assert_eq!(
            TWO_VCPU_CHECKPOINT_OWNERSHIP_SET,
            [
                GuestPhysAddr::new(0x30000),
                GuestPhysAddr::new(0x1fc000),
                GuestPhysAddr::new(0x1fd000),
            ]
        );
        assert_eq!(TWO_VCPU_CHECKPOINT_FIRST_STACK - 8, 0x1fdff0);
        assert_eq!(TWO_VCPU_CHECKPOINT_SECOND_STACK - 8, 0x1fcff0);
    }
}
