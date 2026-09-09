use super::{VcpuStateSnapshot, VcpuStateSnapshotComparison};
use crate::config::VmConfig;
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

pub const BOUNDED_CHECKPOINT_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x30000);
pub const BOUNDED_CHECKPOINT_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x10000);
pub const BOUNDED_CHECKPOINT_STACK: u64 = 0x1ff000;
pub const BOUNDED_CHECKPOINT_MARKER: u8 = 0x5a;
pub const BOUNDED_CHECKPOINT_CORRUPTION: u8 = 0xa5;
pub const BOUNDED_CHECKPOINT_PROOF: &[u8; 1] = b"R";
pub const BOUNDED_CHECKPOINT_CAPTURE_RIP: u64 = 0x10009;
pub const BOUNDED_CHECKPOINT_TERMINAL_RIP: u64 = 0x1000e;

const BOUNDED_CHECKPOINT_GUEST_BYTES: [u8; 14] = [
    0xc6, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00, BOUNDED_CHECKPOINT_MARKER, // mov byte [0x30000], 0x5a
    0xf4, // checkpoint HLT
    0xb0, b'R', // mov al, 'R'
    0xe6, 0xe9, // out 0xe9, al
    0xf4, // terminal HLT
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedVcpuPageCheckpoint {
    page_address: GuestPhysAddr,
    page: Vec<u8>,
    vcpu: VcpuStateSnapshot,
}

impl BoundedVcpuPageCheckpoint {
    pub fn capture(
        vcpu: &Vcpu,
        msr_policy: &GuestMsrAccessPolicy,
        memory: &GuestMemory,
        page_address: GuestPhysAddr,
    ) -> Result<Self, Error> {
        if page_address.get() % LONG_MODE_PAGE_SIZE != 0 {
            return Err(checkpoint_verification_error(
                "bounded checkpoint capture",
                format!("checkpoint page {:#x} is not 4KiB aligned", page_address.get()),
            ));
        }

        let mut page = vec![0_u8; LONG_MODE_PAGE_SIZE as usize];
        memory.read(page_address, &mut page)?;
        let vcpu = vcpu.capture_state_snapshot(msr_policy)?;
        Ok(Self {
            page_address,
            page,
            vcpu,
        })
    }

    #[must_use]
    pub const fn page_address(&self) -> GuestPhysAddr {
        self.page_address
    }

    #[must_use]
    pub fn page(&self) -> &[u8] {
        &self.page
    }

    #[must_use]
    pub const fn vcpu(&self) -> &VcpuStateSnapshot {
        &self.vcpu
    }

    pub fn verify(
        &self,
        vcpu: &Vcpu,
        memory: &GuestMemory,
    ) -> Result<BoundedCheckpointComparison, Error> {
        let mut observed_page = vec![0_u8; self.page.len()];
        memory.read(self.page_address, &mut observed_page)?;
        let vcpu = vcpu.verify_state_snapshot(&self.vcpu)?;
        Ok(BoundedCheckpointComparison {
            page_exact: observed_page == self.page,
            vcpu,
        })
    }

    pub fn restore_and_verify(
        &self,
        vcpu: &Vcpu,
        memory: &mut GuestMemory,
    ) -> Result<BoundedCheckpointComparison, Error> {
        memory.write(self.page_address, &self.page)?;
        let vcpu = vcpu.restore_and_verify_state_snapshot(&self.vcpu)?;
        let mut observed_page = vec![0_u8; self.page.len()];
        memory.read(self.page_address, &mut observed_page)?;
        Ok(BoundedCheckpointComparison {
            page_exact: observed_page == self.page,
            vcpu,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedCheckpointComparison {
    page_exact: bool,
    vcpu: VcpuStateSnapshotComparison,
}

impl BoundedCheckpointComparison {
    #[must_use]
    pub const fn page_exact(&self) -> bool {
        self.page_exact
    }

    #[must_use]
    pub const fn vcpu(&self) -> &VcpuStateSnapshotComparison {
        &self.vcpu
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.page_exact && self.vcpu.is_exact_match()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedCheckpointGuestResult {
    checkpoint_report: VmExitReport,
    corruption: BoundedCheckpointComparison,
    restored: BoundedCheckpointComparison,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    terminal_report: VmExitReport,
    restored_marker: u8,
}

impl BoundedCheckpointGuestResult {
    #[must_use]
    pub const fn checkpoint_report(&self) -> VmExitReport {
        self.checkpoint_report
    }

    #[must_use]
    pub const fn corruption(&self) -> &BoundedCheckpointComparison {
        &self.corruption
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedCheckpointComparison {
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

    #[must_use]
    pub const fn restored_marker(&self) -> u8 {
        self.restored_marker
    }
}

pub fn run_bounded_checkpoint_guest(
    config: VmConfig,
) -> Result<BoundedCheckpointGuestResult, Error> {
    let image = FlatGuestImage::new(
        BOUNDED_CHECKPOINT_ENTRY,
        BOUNDED_CHECKPOINT_ENTRY,
        &BOUNDED_CHECKPOINT_GUEST_BYTES,
    )?;
    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeBootLayout::new(
        memory.region(),
        image.entry(),
        BOUNDED_CHECKPOINT_STACK,
    )
    .expect("fixed bounded checkpoint layout remains valid");
    layout.install_page_tables(&mut memory)?;
    image.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode(&layout)?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty checkpoint MSR policy is valid by construction");

    let mut no_io = PortIoBus::empty();
    let checkpoint_execution = run_vcpu_until_stopped(&mut vcpu, &mut no_io, 1)?;
    let checkpoint_report = checkpoint_execution.report();
    require_hlt_report(
        "bounded checkpoint capture boundary",
        checkpoint_report,
        BOUNDED_CHECKPOINT_CAPTURE_RIP,
    )?;
    if !checkpoint_execution.io_exits().is_empty() {
        return Err(checkpoint_verification_error(
            "bounded checkpoint capture boundary",
            "checkpoint boundary unexpectedly serviced port I/O",
        ));
    }

    let checkpoint = BoundedVcpuPageCheckpoint::capture(
        &vcpu,
        &msr_policy,
        vm.guest_memory()
            .expect("registered checkpoint memory remains VM-owned"),
        BOUNDED_CHECKPOINT_PAGE,
    )?;
    if checkpoint.page().first().copied() != Some(BOUNDED_CHECKPOINT_MARKER) {
        return Err(checkpoint_verification_error(
            "bounded checkpoint page capture",
            format!(
                "expected checkpoint marker {BOUNDED_CHECKPOINT_MARKER:#x}, got {:?}",
                checkpoint.page().first().copied()
            ),
        ));
    }

    let corrupt_page = vec![BOUNDED_CHECKPOINT_CORRUPTION; LONG_MODE_PAGE_SIZE as usize];
    vm.guest_memory_mut()
        .expect("registered checkpoint memory remains VM-owned")
        .write(BOUNDED_CHECKPOINT_PAGE, &corrupt_page)?;
    vcpu.initialize_real_mode(GuestPhysAddr::new(0x100))?;
    let corruption = checkpoint.verify(
        &vcpu,
        vm.guest_memory()
            .expect("registered checkpoint memory remains VM-owned"),
    )?;
    if corruption.is_exact_match() || corruption.page_exact() || corruption.vcpu().is_exact_match() {
        return Err(checkpoint_verification_error(
            "bounded checkpoint corruption proof",
            "intentional page and vCPU corruption did not produce both expected mismatches",
        ));
    }

    let restored = checkpoint.restore_and_verify(
        &vcpu,
        vm.guest_memory_mut()
            .expect("registered checkpoint memory remains VM-owned"),
    )?;
    if !restored.is_exact_match() {
        return Err(checkpoint_verification_error(
            "bounded checkpoint restore verification",
            format!(
                "restored checkpoint mismatch: page_exact={}, vcpu_exact={}",
                restored.page_exact(),
                restored.vcpu().is_exact_match()
            ),
        ));
    }

    let mut restored_marker = [0_u8; 1];
    vm.guest_memory()
        .expect("registered checkpoint memory remains VM-owned")
        .read(BOUNDED_CHECKPOINT_PAGE, &mut restored_marker)?;

    let mut port_io = PortIoBus::with_debug_port();
    let resumed = run_vcpu_until_stopped(&mut vcpu, &mut port_io, 2)?;
    let terminal_report = resumed.report();
    require_hlt_report(
        "bounded checkpoint resumed terminal",
        terminal_report,
        BOUNDED_CHECKPOINT_TERMINAL_RIP,
    )?;
    if resumed.io_exits().len() != BOUNDED_CHECKPOINT_PROOF.len() {
        return Err(checkpoint_verification_error(
            "bounded checkpoint resumed proof",
            format!(
                "expected {} debug-port exit, got {}",
                BOUNDED_CHECKPOINT_PROOF.len(),
                resumed.io_exits().len()
            ),
        ));
    }
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != BOUNDED_CHECKPOINT_PROOF {
        return Err(checkpoint_verification_error(
            "bounded checkpoint resumed proof",
            format!("expected {:?}, got {proof:?}", BOUNDED_CHECKPOINT_PROOF),
        ));
    }
    for (io, expected) in resumed
        .io_exits()
        .iter()
        .zip(BOUNDED_CHECKPOINT_PROOF.iter().copied())
    {
        if io.direction() != PortIoDirection::Out
            || io.port() != DEBUG_PORT
            || io.size() != 1
            || io.count() != 1
            || io.output_data() != [expected]
        {
            return Err(checkpoint_verification_error(
                "bounded checkpoint resumed I/O",
                format!("unexpected debug-port exit {io:?}"),
            ));
        }
    }

    Ok(BoundedCheckpointGuestResult {
        checkpoint_report,
        corruption,
        restored,
        io_exits: resumed.io_exits().to_vec(),
        proof,
        terminal_report,
        restored_marker: restored_marker[0],
    })
}

fn require_hlt_report(
    operation: &'static str,
    report: VmExitReport,
    expected_rip: u64,
) -> Result<(), Error> {
    if report.exit() != VcpuExit::Hlt || report.rip() != expected_rip || report.rflags() & 0x2 != 0x2 {
        return Err(checkpoint_verification_error(
            operation,
            format!(
                "expected HLT at rip={expected_rip:#x} with architectural RFLAGS bit1, got {report}"
            ),
        ));
    }
    Ok(())
}

fn checkpoint_verification_error(operation: &'static str, detail: impl Into<String>) -> Error {
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
    fn deterministic_checkpoint_guest_has_two_hlt_boundaries_and_one_resume_byte() {
        assert_eq!(BOUNDED_CHECKPOINT_GUEST_BYTES.len(), 14);
        assert_eq!(BOUNDED_CHECKPOINT_GUEST_BYTES[7], BOUNDED_CHECKPOINT_MARKER);
        assert_eq!(BOUNDED_CHECKPOINT_GUEST_BYTES[8], 0xf4);
        assert_eq!(&BOUNDED_CHECKPOINT_GUEST_BYTES[9..13], &[0xb0, b'R', 0xe6, 0xe9]);
        assert_eq!(BOUNDED_CHECKPOINT_GUEST_BYTES[13], 0xf4);
        assert_eq!(BOUNDED_CHECKPOINT_CAPTURE_RIP, BOUNDED_CHECKPOINT_ENTRY.get() + 9);
        assert_eq!(BOUNDED_CHECKPOINT_TERMINAL_RIP, BOUNDED_CHECKPOINT_ENTRY.get() + 14);
    }
}
