use super::{
    page_set_error, BoundedTwoVcpuPageSetCheckpoint,
    BoundedTwoVcpuPageSetCheckpointComparison,
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
use crate::vcpu::{PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use crate::vmexit::VmExitReport;

pub const TWO_VCPU_CHECKPOINT_FIRST_ID: VcpuId = VcpuId::BOOT;
pub const TWO_VCPU_CHECKPOINT_SECOND_ID: VcpuId = VcpuId::new(1);
pub const TWO_VCPU_CHECKPOINT_FIRST_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x10000);
pub const TWO_VCPU_CHECKPOINT_SECOND_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x11000);
pub const TWO_VCPU_CHECKPOINT_SHARED_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x30000);
pub const TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x1fc000);
pub const TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x1fd000);
pub const TWO_VCPU_CHECKPOINT_FIRST_STACK_POINTER: u64 = 0x1fcff8;
pub const TWO_VCPU_CHECKPOINT_SECOND_STACK_POINTER: u64 = 0x1fdff8;
pub const TWO_VCPU_CHECKPOINT_FIRST_STACK_VALUE: GuestPhysAddr = GuestPhysAddr::new(0x1fcff0);
pub const TWO_VCPU_CHECKPOINT_SECOND_STACK_VALUE: GuestPhysAddr = GuestPhysAddr::new(0x1fdff0);
pub const TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP: u64 = 0x1000b;
pub const TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP: u64 = 0x1100b;
pub const TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP: u64 = 0x1001c;
pub const TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP: u64 = 0x1101c;
pub const TWO_VCPU_CHECKPOINT_FIRST_PROOF: &[u8; 3] = b"AX0";
pub const TWO_VCPU_CHECKPOINT_SECOND_PROOF: &[u8; 3] = b"BY1";
pub const TWO_VCPU_CHECKPOINT_OWNERSHIP_SET: [GuestPhysAddr; 3] = [
    TWO_VCPU_CHECKPOINT_SHARED_PAGE,
    TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE,
    TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
];

const FIRST_MARKER: u8 = b'A';
const SECOND_MARKER: u8 = b'B';
const FIRST_STACK_MARKER: u8 = b'X';
const SECOND_STACK_MARKER: u8 = b'Y';
const FIRST_CORRUPT_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x12000);
const SECOND_CORRUPT_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x13000);
const FIRST_CORRUPT_STACK: u64 = 0x1faff8;
const SECOND_CORRUPT_STACK: u64 = 0x1fbff8;

const FIRST_GUEST_BYTES: [u8; 28] = [
    0xc6, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00, FIRST_MARKER, // mov byte [0x30000], 'A'
    0x6a, FIRST_STACK_MARKER, // push 'X'
    0xf4, // capture HLT
    0x8a, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00, // mov al, [0x30000]
    0xe6, 0xe9, // out A
    0x58, // pop rax => X
    0xe6, 0xe9, // out X
    0xb0, b'0', // mov al, '0'
    0xe6, 0xe9, // out 0
    0xf4, // terminal HLT
];

const SECOND_GUEST_BYTES: [u8; 28] = [
    0xc6, 0x04, 0x25, 0x01, 0x00, 0x03, 0x00, SECOND_MARKER, // mov byte [0x30001], 'B'
    0x6a, SECOND_STACK_MARKER, // push 'Y'
    0xf4, // capture HLT
    0x8a, 0x04, 0x25, 0x01, 0x00, 0x03, 0x00, // mov al, [0x30001]
    0xe6, 0xe9, // out B
    0x58, // pop rax => Y
    0xe6, 0xe9, // out Y
    0xb0, b'1', // mov al, '1'
    0xe6, 0xe9, // out 1
    0xf4, // terminal HLT
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoVcpuCheckpointGuestResult {
    first_capture: VmExitReport,
    second_capture: VmExitReport,
    captured_pages: Vec<GuestPhysAddr>,
    corruption: BoundedTwoVcpuPageSetCheckpointComparison,
    restored: BoundedTwoVcpuPageSetCheckpointComparison,
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
    pub const fn corruption(&self) -> &BoundedTwoVcpuPageSetCheckpointComparison {
        &self.corruption
    }
    #[must_use]
    pub const fn restored(&self) -> &BoundedTwoVcpuPageSetCheckpointComparison {
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

pub fn run_two_vcpu_quiescent_checkpoint_guest(
    config: VmConfig,
) -> Result<TwoVcpuCheckpointGuestResult, Error> {
    if config.vcpu_count() != 1 {
        return Err(page_set_error(
            "two-vCPU checkpoint fixture configuration",
            "fixture owns exactly two VCPUs internally and requires the default one-vCPU config",
        ));
    }

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
        TWO_VCPU_CHECKPOINT_FIRST_STACK_POINTER,
    )
    .expect("fixed first two-vCPU checkpoint layout remains valid");
    let second_layout = LongModeBootLayout::new(
        memory.region(),
        second_image.entry(),
        TWO_VCPU_CHECKPOINT_SECOND_STACK_POINTER,
    )
    .expect("fixed second two-vCPU checkpoint layout remains valid");
    let first_corrupt_layout = LongModeBootLayout::new(
        memory.region(),
        FIRST_CORRUPT_ENTRY,
        FIRST_CORRUPT_STACK,
    )
    .expect("fixed first corruption layout remains valid");
    let second_corrupt_layout = LongModeBootLayout::new(
        memory.region(),
        SECOND_CORRUPT_ENTRY,
        SECOND_CORRUPT_STACK,
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

    let worker = std::thread::spawn(move || -> Result<(Vcpu, VmExitReport), Error> {
        let mut no_io = PortIoBus::empty();
        let execution = run_vcpu_until_stopped(&mut second_vcpu, &mut no_io, 1)?;
        let report = execution.report();
        require_hlt_report(
            "two-vCPU AP capture boundary",
            report,
            TWO_VCPU_CHECKPOINT_SECOND_ID,
            TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP,
        )?;
        if !execution.io_exits().is_empty() || !execution.mmio_exits().is_empty() {
            return Err(page_set_error(
                "two-vCPU AP capture boundary",
                "capture boundary unexpectedly serviced PIO/MMIO",
            ));
        }
        Ok((second_vcpu, report))
    });

    let mut no_io = PortIoBus::empty();
    let first_execution = run_vcpu_until_stopped(&mut first_vcpu, &mut no_io, 1);
    let second_result = worker.join().map_err(|_| {
        page_set_error(
            "two-vCPU checkpoint quiescence join",
            "AP worker panicked before returning VCPU ownership",
        )
    })?;
    let first_execution = first_execution?;
    let (mut second_vcpu, second_capture) = second_result?;
    let first_capture = first_execution.report();
    require_hlt_report(
        "two-vCPU BSP capture boundary",
        first_capture,
        TWO_VCPU_CHECKPOINT_FIRST_ID,
        TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP,
    )?;
    if !first_execution.io_exits().is_empty() || !first_execution.mmio_exits().is_empty() {
        return Err(page_set_error(
            "two-vCPU BSP capture boundary",
            "capture boundary unexpectedly serviced PIO/MMIO",
        ));
    }

    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty two-vCPU checkpoint MSR policy is valid by construction");
    let checkpoint = BoundedTwoVcpuPageSetCheckpoint::capture(
        [&first_vcpu, &second_vcpu],
        &msr_policy,
        vm.guest_memory()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
        &TWO_VCPU_CHECKPOINT_OWNERSHIP_SET,
    )?;
    require_captured_roles(&checkpoint)?;
    let captured_pages = checkpoint
        .pages()
        .iter()
        .map(|page| page.address())
        .collect::<Vec<_>>();

    corrupt_owned_pages(
        vm.guest_memory_mut()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
    )?;
    first_vcpu.initialize_long_mode(&first_corrupt_layout)?;
    second_vcpu.initialize_long_mode(&second_corrupt_layout)?;

    let corruption = checkpoint.verify(
        [&first_vcpu, &second_vcpu],
        vm.guest_memory()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
    )?;
    require_corruption(&corruption)?;

    let restored = checkpoint.restore_and_verify(
        [&first_vcpu, &second_vcpu],
        vm.guest_memory_mut()
            .expect("registered two-vCPU checkpoint memory remains VM-owned"),
    )?;
    if !restored.is_exact_match() {
        return Err(page_set_error(
            "two-vCPU checkpoint restore verification",
            format!(
                "restore mismatch: pages={:?} vcpus={:?}",
                restored.pages(),
                restored.vcpus()
            ),
        ));
    }

    let (first_io_exits, first_proof, first_terminal) = resume_and_prove(
        &mut first_vcpu,
        TWO_VCPU_CHECKPOINT_FIRST_PROOF,
        TWO_VCPU_CHECKPOINT_FIRST_ID,
        TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP,
        "two-vCPU BSP resumed proof",
    )?;
    let (second_io_exits, second_proof, second_terminal) = resume_and_prove(
        &mut second_vcpu,
        TWO_VCPU_CHECKPOINT_SECOND_PROOF,
        TWO_VCPU_CHECKPOINT_SECOND_ID,
        TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP,
        "two-vCPU AP resumed proof",
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

fn resume_and_prove(
    vcpu: &mut Vcpu,
    expected_proof: &[u8],
    expected_id: VcpuId,
    expected_terminal_rip: u64,
    operation: &'static str,
) -> Result<(Vec<PortIoExit>, Vec<u8>, VmExitReport), Error> {
    let mut port_io = PortIoBus::with_debug_port();
    let execution = run_vcpu_until_stopped(vcpu, &mut port_io, 4)?;
    let report = execution.report();
    require_hlt_report(operation, report, expected_id, expected_terminal_rip)?;
    if execution.io_exits().len() != expected_proof.len() || !execution.mmio_exits().is_empty() {
        return Err(page_set_error(
            operation,
            format!(
                "expected {} PIO exits and zero MMIO exits, got {}/{}",
                expected_proof.len(),
                execution.io_exits().len(),
                execution.mmio_exits().len()
            ),
        ));
    }
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof != expected_proof {
        return Err(page_set_error(
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
            return Err(page_set_error(
                operation,
                format!("unexpected debug-port exit {io:?}"),
            ));
        }
    }
    Ok((execution.io_exits().to_vec(), proof, report))
}

fn require_captured_roles(checkpoint: &BoundedTwoVcpuPageSetCheckpoint) -> Result<(), Error> {
    let shared = checkpoint
        .page(TWO_VCPU_CHECKPOINT_SHARED_PAGE)
        .ok_or_else(|| page_set_error("two-vCPU role capture", "missing shared page"))?;
    let first_stack = checkpoint
        .page(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE)
        .ok_or_else(|| page_set_error("two-vCPU role capture", "missing BSP stack page"))?;
    let second_stack = checkpoint
        .page(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE)
        .ok_or_else(|| page_set_error("two-vCPU role capture", "missing AP stack page"))?;
    let first_stack_offset = usize::try_from(
        TWO_VCPU_CHECKPOINT_FIRST_STACK_VALUE.get() - TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE.get(),
    )
    .expect("fixed BSP stack marker offset fits usize");
    let second_stack_offset = usize::try_from(
        TWO_VCPU_CHECKPOINT_SECOND_STACK_VALUE.get() - TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE.get(),
    )
    .expect("fixed AP stack marker offset fits usize");
    if shared.bytes().first().copied() != Some(FIRST_MARKER)
        || shared.bytes().get(1).copied() != Some(SECOND_MARKER)
        || first_stack.bytes().get(first_stack_offset).copied() != Some(FIRST_STACK_MARKER)
        || second_stack.bytes().get(second_stack_offset).copied() != Some(SECOND_STACK_MARKER)
    {
        return Err(page_set_error(
            "two-vCPU role capture",
            "captured shared/BSP-stack/AP-stack markers do not match A/B/X/Y",
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

fn require_corruption(
    comparison: &BoundedTwoVcpuPageSetCheckpointComparison,
) -> Result<(), Error> {
    for page in TWO_VCPU_CHECKPOINT_OWNERSHIP_SET {
        if comparison.page_exact(page) != Some(false) {
            return Err(page_set_error(
                "two-vCPU checkpoint corruption proof",
                format!("owned page {:#x} did not prove mismatch", page.get()),
            ));
        }
    }
    for id in [TWO_VCPU_CHECKPOINT_FIRST_ID, TWO_VCPU_CHECKPOINT_SECOND_ID] {
        if comparison.vcpu_exact(id) != Some(false) {
            return Err(page_set_error(
                "two-vCPU checkpoint corruption proof",
                format!("vCPU {} did not independently prove mismatch", id.get()),
            ));
        }
    }
    Ok(())
}

fn require_hlt_report(
    operation: &'static str,
    report: VmExitReport,
    expected_id: VcpuId,
    expected_rip: u64,
) -> Result<(), Error> {
    if report.vcpu_id() != expected_id
        || report.exit() != VcpuExit::Hlt
        || report.rip() != expected_rip
        || report.rflags() & 0x2 != 0x2
    {
        return Err(page_set_error(
            operation,
            format!(
                "expected vCPU {} HLT at rip={expected_rip:#x} with RFLAGS bit1, got {report}",
                expected_id.get()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_programs_bind_shared_and_private_stack_roles() {
        assert_eq!(FIRST_GUEST_BYTES.len(), 28);
        assert_eq!(SECOND_GUEST_BYTES.len(), 28);
        assert_eq!(FIRST_GUEST_BYTES[10], 0xf4);
        assert_eq!(SECOND_GUEST_BYTES[10], 0xf4);
        assert_eq!(FIRST_GUEST_BYTES[27], 0xf4);
        assert_eq!(SECOND_GUEST_BYTES[27], 0xf4);
        assert_eq!(
            TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP,
            TWO_VCPU_CHECKPOINT_FIRST_ENTRY.get() + 11
        );
        assert_eq!(
            TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP,
            TWO_VCPU_CHECKPOINT_SECOND_ENTRY.get() + 11
        );
        assert_eq!(
            TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP,
            TWO_VCPU_CHECKPOINT_FIRST_ENTRY.get() + 28
        );
        assert_eq!(
            TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP,
            TWO_VCPU_CHECKPOINT_SECOND_ENTRY.get() + 28
        );
    }
}
