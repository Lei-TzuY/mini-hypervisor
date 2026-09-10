use crate::config::VmConfig;
use crate::execution::run_vcpu_until_stopped_with_mmio_observer;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use crate::mmio::long_mode::{
    LongModeMmioBootLayout, LONG_MODE_MMIO_GUEST_ENTRY, LONG_MODE_MMIO_STACK_POINTER,
};
use crate::mmio::{MmioBus, MmioDeviceEvent, MmioDeviceEventRecord};
use crate::portio::pci::virtio::{
    VIRTIO_F_VERSION_1, VIRTIO_ISR_OFFSET, VIRTIO_STATUS_ACKNOWLEDGE, VIRTIO_STATUS_DRIVER,
    VIRTIO_STATUS_DRIVER_OK, VIRTIO_STATUS_FEATURES_OK,
};
use crate::portio::pci::virtio_blk::{
    VirtioBlkQueueCompletion, VIRTIO_BLK_CAPACITY_SECTORS, VIRTIO_BLK_SECTOR_SIZE,
    VIRTIO_BLK_S_OK, VIRTIO_BLK_T_IN, VIRTIO_BLK_T_OUT, VIRTQ_DESC_F_NEXT,
    VIRTQ_DESC_F_WRITE,
};
use crate::portio::virtio_blk_fixture::{
    deterministic_write_readback_sector, VIRTIO_BLK_AVAIL_GPA, VIRTIO_BLK_BAR0_GPA,
    VIRTIO_BLK_DATA_GPA, VIRTIO_BLK_DESCRIPTOR_GPA, VIRTIO_BLK_HEADER_GPA,
    VIRTIO_BLK_STATUS_GPA, VIRTIO_BLK_USED_GPA,
};
use crate::portio::{PortIoBus, DEBUG_PORT};
use crate::vcpu::{MmioDirection, PortIoDirection, PortIoExit, VcpuExit};
use crate::vmexit::{VmExitContinuation, VmExitReport};

pub const VIRTIO_BLK_CHECKPOINT_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x0001_8000);
pub const VIRTIO_BLK_CHECKPOINT_CAPTURE_PROOF: &[u8; 3] = b"BWO";
pub const VIRTIO_BLK_CHECKPOINT_REPLAY_PROOF: &[u8; 3] = b"NRD";

const VIRTIO_BLK_CHECKPOINT_FIRST_EXIT_BUDGET: u32 = 24;
const VIRTIO_BLK_CHECKPOINT_REPLAY_EXIT_BUDGET: u32 = 6;
const VIRTIO_BLK_CHECKPOINT_QUEUE_SIZE: u16 = 4;
const VIRTIO_BLK_CHECKPOINT_QUEUE_INDEX: u16 = 0;
const VIRTIO_BLK_CHECKPOINT_WRITE_BARRIER: u8 = b'W';
const VIRTIO_BLK_CHECKPOINT_READ_BARRIER: u8 = b'N';
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioBlkCheckpointGuestResult {
    capture_report: VmExitReport,
    capture_proof: Vec<u8>,
    mutation_report: VmExitReport,
    mutation_proof: Vec<u8>,
    replay_report: VmExitReport,
    replay_proof: Vec<u8>,
    captured_avail_idx: u16,
    captured_used_idx: u16,
    mutation: BoundedVirtioBlkCheckpointComparison,
    restored: BoundedVirtioBlkCheckpointComparison,
    replay_avail_idx: u16,
    replay_used_idx: u16,
    backing: Vec<u8>,
    readback: Vec<u8>,
}

impl VirtioBlkCheckpointGuestResult {
    #[must_use]
    pub const fn capture_report(&self) -> VmExitReport {
        self.capture_report
    }

    #[must_use]
    pub fn capture_proof(&self) -> &[u8] {
        &self.capture_proof
    }

    #[must_use]
    pub const fn mutation_report(&self) -> VmExitReport {
        self.mutation_report
    }

    #[must_use]
    pub fn mutation_proof(&self) -> &[u8] {
        &self.mutation_proof
    }

    #[must_use]
    pub const fn replay_report(&self) -> VmExitReport {
        self.replay_report
    }

    #[must_use]
    pub fn replay_proof(&self) -> &[u8] {
        &self.replay_proof
    }

    #[must_use]
    pub const fn captured_avail_idx(&self) -> u16 {
        self.captured_avail_idx
    }

    #[must_use]
    pub const fn captured_used_idx(&self) -> u16 {
        self.captured_used_idx
    }

    #[must_use]
    pub const fn mutation(&self) -> &BoundedVirtioBlkCheckpointComparison {
        &self.mutation
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedVirtioBlkCheckpointComparison {
        &self.restored
    }

    #[must_use]
    pub const fn replay_avail_idx(&self) -> u16 {
        self.replay_avail_idx
    }

    #[must_use]
    pub const fn replay_used_idx(&self) -> u16 {
        self.replay_used_idx
    }

    #[must_use]
    pub fn backing(&self) -> &[u8] {
        &self.backing
    }

    #[must_use]
    pub fn readback(&self) -> &[u8] {
        &self.readback
    }
}

struct VirtioBlkCheckpointProgram {
    bytes: Vec<u8>,
    checkpoint_rip: u64,
    terminal_rip: u64,
}

pub fn run_virtio_blk_checkpoint_guest(
    config: VmConfig,
) -> Result<VirtioBlkCheckpointGuestResult, Error> {
    let program = build_virtio_blk_checkpoint_guest();
    let image = FlatGuestImage::new(
        LONG_MODE_MMIO_GUEST_ENTRY,
        LONG_MODE_MMIO_GUEST_ENTRY,
        &program.bytes,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout =
        LongModeMmioBootLayout::new(memory.region(), image.entry(), LONG_MODE_MMIO_STACK_POINTER)
            .expect("fixed virtio-blk checkpoint BAR mapping remains valid");
    layout.install_page_tables(&mut memory)?;
    image.load(&mut memory)?;
    let write_sector = deterministic_write_readback_sector();
    memory.write(GuestPhysAddr::new(VIRTIO_BLK_DATA_GPA), &write_sector)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode(layout.boot_layout())?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty virtio-blk checkpoint MSR policy is valid by construction");

    let mut mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(VIRTIO_BLK_BAR0_GPA)
        .expect("fixed virtio-blk checkpoint BAR does not overlap another MMIO device");

    let mut first_completion = None;
    let mut capture_io = PortIoBus::with_debug_port();
    let capture_execution = run_vcpu_until_stopped_with_mmio_observer(
        &mut vcpu,
        &mut capture_io,
        &mut mmio,
        VIRTIO_BLK_CHECKPOINT_FIRST_EXIT_BUDGET,
        |continuation, mmio| {
            process_checkpoint_notification(
                continuation,
                mmio,
                &mut vm,
                VIRTIO_BLK_CHECKPOINT_WRITE_BARRIER,
                &mut first_completion,
            )
        },
    )?;
    require_checkpoint_hlt(
        "virtio-blk checkpoint capture boundary",
        capture_execution.report(),
        program.checkpoint_rip,
    )?;
    validate_debug_proof(
        "virtio-blk checkpoint capture proof",
        capture_execution.io_exits(),
        VIRTIO_BLK_CHECKPOINT_CAPTURE_PROOF,
    )?;
    validate_capture_mmio(capture_execution.mmio_exits())?;
    let first_completion = first_completion.ok_or_else(|| {
        virtio_blk_checkpoint_guest_error("first T_OUT request was never processed")
    })?;
    if first_completion.descriptor_id() != 0
        || first_completion.length() != 1
        || first_completion.sector() != 0
    {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "unexpected first T_OUT completion: {first_completion:?}"
        )));
    }
    require_no_device_event(&mut mmio, "capture boundary")?;
    if mmio.virtio_blk_sector_at(VIRTIO_BLK_BAR0_GPA) != Some(&write_sector) {
        return Err(virtio_blk_checkpoint_guest_error(
            "T_OUT backing was not committed before checkpoint capture",
        ));
    }

    let checkpoint = BoundedVirtioBlkCheckpoint::capture(
        &vcpu,
        &msr_policy,
        vm.guest_memory()
            .expect("registered virtio-blk checkpoint memory remains VM-owned"),
        &mmio,
        VIRTIO_BLK_BAR0_GPA,
        &[VIRTIO_BLK_CHECKPOINT_PAGE],
    )?;
    if checkpoint.pages().len() != 1
        || checkpoint.pages()[0].address() != VIRTIO_BLK_CHECKPOINT_PAGE
        || !checkpoint.device().checkpoint_quiescent()
        || checkpoint.device().checkpoint_last_avail_idx() != 1
        || checkpoint.device().checkpoint_last_used_idx() != 1
        || checkpoint.device().sector0() != &write_sector
    {
        return Err(virtio_blk_checkpoint_guest_error(
            "captured checkpoint did not own the expected quiescent queue/backing state",
        ));
    }
    let captured_avail_idx = checkpoint.device().checkpoint_last_avail_idx();
    let captured_used_idx = checkpoint.device().checkpoint_last_used_idx();
    let capture_proof = capture_io.debug_output().unwrap_or(&[]).to_vec();
    let capture_report = capture_execution.report();

    let mut mutation_completion = None;
    let mut mutation_io = PortIoBus::with_debug_port();
    let mutation_execution = run_vcpu_until_stopped_with_mmio_observer(
        &mut vcpu,
        &mut mutation_io,
        &mut mmio,
        VIRTIO_BLK_CHECKPOINT_REPLAY_EXIT_BUDGET,
        |continuation, mmio| {
            process_checkpoint_notification(
                continuation,
                mmio,
                &mut vm,
                VIRTIO_BLK_CHECKPOINT_READ_BARRIER,
                &mut mutation_completion,
            )
        },
    )?;
    require_checkpoint_hlt(
        "virtio-blk checkpoint mutation terminal",
        mutation_execution.report(),
        program.terminal_rip,
    )?;
    validate_debug_proof(
        "virtio-blk checkpoint mutation proof",
        mutation_execution.io_exits(),
        VIRTIO_BLK_CHECKPOINT_REPLAY_PROOF,
    )?;
    validate_replay_mmio(mutation_execution.mmio_exits())?;
    require_read_completion(mutation_completion, "post-capture mutation")?;
    require_no_device_event(&mut mmio, "post-capture mutation terminal")?;

    let mutation = checkpoint.verify(
        &vcpu,
        vm.guest_memory()
            .expect("registered virtio-blk checkpoint memory remains VM-owned"),
        &mmio,
    )?;
    if mutation.page_exact(VIRTIO_BLK_CHECKPOINT_PAGE) != Some(false)
        || mutation.vcpu_exact()
        || mutation.device_exact()
    {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "legitimate T_IN mutation did not produce a full fresh mismatch: page={:?} vcpu={} device={}",
            mutation.page_exact(VIRTIO_BLK_CHECKPOINT_PAGE),
            mutation.vcpu_exact(),
            mutation.device_exact()
        )));
    }
    let mutation_device = capture_quiescent_device(&mmio, "post-capture mutation")?;
    if mutation_device.checkpoint_last_avail_idx() != 2
        || mutation_device.checkpoint_last_used_idx() != 2
    {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "post-capture request did not advance queue indices to 2/2: {}/{}",
            mutation_device.checkpoint_last_avail_idx(),
            mutation_device.checkpoint_last_used_idx()
        )));
    }
    let mutation_proof = mutation_io.debug_output().unwrap_or(&[]).to_vec();
    let mutation_report = mutation_execution.report();

    let restored = checkpoint.restore_and_verify(
        &vcpu,
        vm.guest_memory_mut()
            .expect("registered virtio-blk checkpoint memory remains VM-owned"),
        &mut mmio,
    )?;
    if !restored.is_exact_match()
        || restored.page_exact(VIRTIO_BLK_CHECKPOINT_PAGE) != Some(true)
        || !restored.vcpu_exact()
        || !restored.device_exact()
    {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "virtio-blk checkpoint restore was not exact: page={:?} vcpu={} device={}",
            restored.page_exact(VIRTIO_BLK_CHECKPOINT_PAGE),
            restored.vcpu_exact(),
            restored.device_exact()
        )));
    }
    require_no_device_event(&mut mmio, "post-restore boundary")?;
    let restored_device = capture_quiescent_device(&mmio, "post-restore boundary")?;
    if restored_device.checkpoint_last_avail_idx() != 1
        || restored_device.checkpoint_last_used_idx() != 1
        || restored_device.sector0() != &write_sector
    {
        return Err(virtio_blk_checkpoint_guest_error(
            "restored virtio-blk state did not return to captured queue/backing ownership",
        ));
    }

    let mut replay_completion = None;
    let mut replay_io = PortIoBus::with_debug_port();
    let replay_execution = run_vcpu_until_stopped_with_mmio_observer(
        &mut vcpu,
        &mut replay_io,
        &mut mmio,
        VIRTIO_BLK_CHECKPOINT_REPLAY_EXIT_BUDGET,
        |continuation, mmio| {
            process_checkpoint_notification(
                continuation,
                mmio,
                &mut vm,
                VIRTIO_BLK_CHECKPOINT_READ_BARRIER,
                &mut replay_completion,
            )
        },
    )?;
    require_checkpoint_hlt(
        "virtio-blk checkpoint replay terminal",
        replay_execution.report(),
        program.terminal_rip,
    )?;
    validate_debug_proof(
        "virtio-blk checkpoint replay proof",
        replay_execution.io_exits(),
        VIRTIO_BLK_CHECKPOINT_REPLAY_PROOF,
    )?;
    validate_replay_mmio(replay_execution.mmio_exits())?;
    require_read_completion(replay_completion, "post-restore replay")?;
    require_no_device_event(&mut mmio, "post-restore replay terminal")?;

    let replay_device = capture_quiescent_device(&mmio, "post-restore replay terminal")?;
    let replay_avail_idx = replay_device.checkpoint_last_avail_idx();
    let replay_used_idx = replay_device.checkpoint_last_used_idx();
    if replay_avail_idx != 2 || replay_used_idx != 2 {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "replayed request did not advance restored queue from 1/1 to 2/2: {replay_avail_idx}/{replay_used_idx}"
        )));
    }
    let backing = mmio
        .virtio_blk_sector_at(VIRTIO_BLK_BAR0_GPA)
        .ok_or_else(|| virtio_blk_checkpoint_guest_error("virtio-blk backing disappeared"))?
        .to_vec();
    let mut readback = vec![0_u8; VIRTIO_BLK_SECTOR_SIZE];
    vm.guest_memory()
        .expect("registered virtio-blk checkpoint memory remains VM-owned")
        .read(GuestPhysAddr::new(VIRTIO_BLK_DATA_GPA), &mut readback)?;
    if backing.as_slice() != write_sector || readback.as_slice() != write_sector {
        return Err(virtio_blk_checkpoint_guest_error(
            "post-restore request did not preserve checkpointed backing/readback continuity",
        ));
    }

    Ok(VirtioBlkCheckpointGuestResult {
        capture_report,
        capture_proof,
        mutation_report,
        mutation_proof,
        replay_report: replay_execution.report(),
        replay_proof: replay_io.debug_output().unwrap_or(&[]).to_vec(),
        captured_avail_idx,
        captured_used_idx,
        mutation,
        restored,
        replay_avail_idx,
        replay_used_idx,
        backing,
        readback,
    })
}

fn process_checkpoint_notification(
    continuation: &VmExitContinuation,
    mmio: &mut MmioBus,
    vm: &mut crate::kvm::Vm,
    expected_barrier: u8,
    completion: &mut Option<VirtioBlkQueueCompletion>,
) -> Result<(), Error> {
    let Some(barrier) = checkpoint_notify_barrier(continuation) else {
        return Ok(());
    };
    if barrier != expected_barrier || completion.is_some() {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "unexpected or duplicate virtio-blk checkpoint barrier {barrier:#x}, expected {expected_barrier:#x}"
        )));
    }
    let event = mmio.take_device_event_record().ok_or_else(|| {
        virtio_blk_checkpoint_guest_error("checkpoint barrier arrived without a queue event")
    })?;
    let expected_event = MmioDeviceEventRecord::new(
        VIRTIO_BLK_BAR0_GPA,
        MmioDeviceEvent::VirtioQueueNotified {
            queue: VIRTIO_BLK_CHECKPOINT_QUEUE_INDEX,
        },
    );
    if event != expected_event {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "unexpected virtio-blk checkpoint event: {event:?}"
        )));
    }
    let memory = vm.guest_memory_mut().ok_or_else(|| {
        virtio_blk_checkpoint_guest_error("virtio-blk checkpoint VM lost guest memory")
    })?;
    let observed = mmio
        .process_virtio_blk_notification_atomic(VIRTIO_BLK_BAR0_GPA, memory)
        .map_err(|error| {
            virtio_blk_checkpoint_guest_error(format!(
                "virtio-blk checkpoint queue processing failed: {error}"
            ))
        })?
        .ok_or_else(|| {
            virtio_blk_checkpoint_guest_error(
                "virtio-blk checkpoint BAR disappeared before queue processing",
            )
        })?;
    *completion = Some(observed);
    Ok(())
}

fn checkpoint_notify_barrier(continuation: &VmExitContinuation) -> Option<u8> {
    match continuation {
        VmExitContinuation::PortIo(io)
            if io.direction() == PortIoDirection::Out
                && io.port() == DEBUG_PORT
                && io.size() == 1
                && io.count() == 1
                && matches!(
                    io.output_data(),
                    [VIRTIO_BLK_CHECKPOINT_WRITE_BARRIER]
                        | [VIRTIO_BLK_CHECKPOINT_READ_BARRIER]
                ) =>
        {
            Some(io.output_data()[0])
        }
        _ => None,
    }
}

fn require_read_completion(
    completion: Option<VirtioBlkQueueCompletion>,
    operation: &'static str,
) -> Result<(), Error> {
    let completion = completion.ok_or_else(|| {
        virtio_blk_checkpoint_guest_error(format!("{operation}: T_IN request was never processed"))
    })?;
    if completion.descriptor_id() != 0
        || completion.length() != (VIRTIO_BLK_SECTOR_SIZE + 1) as u32
        || completion.sector() != 0
    {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "{operation}: unexpected T_IN completion {completion:?}"
        )));
    }
    Ok(())
}

fn capture_quiescent_device(
    mmio: &MmioBus,
    operation: &'static str,
) -> Result<crate::portio::pci::virtio_blk::VirtioBlkDevice, Error> {
    mmio.capture_virtio_blk_checkpoint_at(VIRTIO_BLK_BAR0_GPA)?
        .ok_or_else(|| {
            virtio_blk_checkpoint_guest_error(format!(
                "{operation}: virtio-blk device disappeared"
            ))
        })
}

fn require_no_device_event(mmio: &mut MmioBus, operation: &'static str) -> Result<(), Error> {
    if let Some(event) = mmio.take_device_event_record() {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "{operation}: unexpected pending device event {event:?}"
        )));
    }
    Ok(())
}

fn require_checkpoint_hlt(
    operation: &'static str,
    report: VmExitReport,
    expected_rip: u64,
) -> Result<(), Error> {
    if report.exit() != VcpuExit::Hlt
        || report.rip() != expected_rip
        || report.rflags() & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
    {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "{operation}: expected HLT at rip={expected_rip:#x} with architectural RFLAGS bit1, got {report}"
        )));
    }
    Ok(())
}

fn validate_debug_proof(
    operation: &'static str,
    exits: &[PortIoExit],
    expected: &[u8],
) -> Result<(), Error> {
    if exits.len() != expected.len() {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "{operation}: expected {} debug exits, got {}",
            expected.len(),
            exits.len()
        )));
    }
    for (exit, byte) in exits.iter().zip(expected.iter().copied()) {
        if exit.direction() != PortIoDirection::Out
            || exit.port() != DEBUG_PORT
            || exit.size() != 1
            || exit.count() != 1
            || exit.output_data() != [byte]
        {
            return Err(virtio_blk_checkpoint_guest_error(format!(
                "{operation}: unexpected debug-port exit {exit:?}"
            )));
        }
    }
    Ok(())
}

fn validate_capture_mmio(exits: &[crate::vcpu::MmioExit]) -> Result<(), Error> {
    if exits.len() != 20 {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "capture phase expected 20 MMIO exits, got {}",
            exits.len()
        )));
    }
    let notify = exits.last().expect("nonempty capture MMIO sequence");
    if notify.address() != VIRTIO_BLK_BAR0_GPA + 0x100
        || notify.direction() != MmioDirection::Write
        || notify.length() != 2
        || notify.write_data() != 0_u16.to_le_bytes()
    {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "capture phase notify MMIO exit mismatch: {notify:?}"
        )));
    }
    Ok(())
}

fn validate_replay_mmio(exits: &[crate::vcpu::MmioExit]) -> Result<(), Error> {
    if exits.len() != 2 {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "replay phase expected 2 MMIO exits, got {}",
            exits.len()
        )));
    }
    let notify = &exits[0];
    let isr = &exits[1];
    if notify.address() != VIRTIO_BLK_BAR0_GPA + 0x100
        || notify.direction() != MmioDirection::Write
        || notify.length() != 2
        || notify.write_data() != 0_u16.to_le_bytes()
        || isr.address() != VIRTIO_BLK_BAR0_GPA + VIRTIO_ISR_OFFSET
        || isr.direction() != MmioDirection::Read
        || isr.length() != 1
        || !isr.write_data().is_empty()
    {
        return Err(virtio_blk_checkpoint_guest_error(format!(
            "replay MMIO sequence mismatch: {exits:?}"
        )));
    }
    Ok(())
}

fn virtio_blk_checkpoint_guest_error(detail: impl Into<String>) -> Error {
    page_set_error("virtio-blk checkpoint proof", detail)
}

fn build_virtio_blk_checkpoint_guest() -> VirtioBlkCheckpointProgram {
    let mut code = Vec::new();

    checkpoint_emit_movabs(&mut code, 3, 0x0050_0000);
    code.extend_from_slice(&[0x48, 0x8b, 0x83, 0x00, 0x03, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0x83, 0xf8, VIRTIO_BLK_CAPACITY_SECTORS as u8]);
    checkpoint_emit_equal_or_ud2(&mut code);
    checkpoint_emit_mmio_byte_write(&mut code, 0x14, VIRTIO_STATUS_ACKNOWLEDGE);
    checkpoint_emit_mmio_byte_write(
        &mut code,
        0x14,
        VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER,
    );
    checkpoint_emit_mmio_dword_write(&mut code, 0x00, 1);
    code.extend_from_slice(&[0x8b, 0x43, 0x04]);
    checkpoint_emit_cmp_eax(&mut code, 1);
    checkpoint_emit_mmio_dword_write(&mut code, 0x08, 1);
    checkpoint_emit_mmio_dword_write(&mut code, 0x0c, (VIRTIO_F_VERSION_1 >> 32) as u32);
    checkpoint_emit_mmio_byte_write(
        &mut code,
        0x14,
        VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK,
    );
    checkpoint_emit_mmio_word_write(&mut code, 0x16, 0);
    checkpoint_emit_mmio_word_write(&mut code, 0x18, VIRTIO_BLK_CHECKPOINT_QUEUE_SIZE);
    checkpoint_emit_mmio_dword_write(&mut code, 0x20, VIRTIO_BLK_DESCRIPTOR_GPA as u32);
    checkpoint_emit_mmio_dword_write(&mut code, 0x24, 0);
    checkpoint_emit_mmio_dword_write(&mut code, 0x28, VIRTIO_BLK_AVAIL_GPA as u32);
    checkpoint_emit_mmio_dword_write(&mut code, 0x2c, 0);
    checkpoint_emit_mmio_dword_write(&mut code, 0x30, VIRTIO_BLK_USED_GPA as u32);
    checkpoint_emit_mmio_dword_write(&mut code, 0x34, 0);
    checkpoint_emit_mmio_word_write(&mut code, 0x1c, 1);
    checkpoint_emit_mmio_byte_write(
        &mut code,
        0x14,
        VIRTIO_STATUS_ACKNOWLEDGE
            | VIRTIO_STATUS_DRIVER
            | VIRTIO_STATUS_FEATURES_OK
            | VIRTIO_STATUS_DRIVER_OK,
    );
    code.extend_from_slice(&[0x8a, 0x43, 0x14]);
    checkpoint_emit_cmp_al(
        &mut code,
        VIRTIO_STATUS_ACKNOWLEDGE
            | VIRTIO_STATUS_DRIVER
            | VIRTIO_STATUS_FEATURES_OK
            | VIRTIO_STATUS_DRIVER_OK,
    );
    checkpoint_emit_debug(&mut code, b'B');

    checkpoint_emit_write_request_setup(&mut code);
    code.extend_from_slice(&[0x66, 0xc7, 0x83, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);
    checkpoint_emit_debug(&mut code, VIRTIO_BLK_CHECKPOINT_WRITE_BARRIER);
    checkpoint_emit_first_completion_checks(&mut code);
    checkpoint_emit_debug(&mut code, b'O');
    code.push(0xf4);
    let checkpoint_rip = LONG_MODE_MMIO_GUEST_ENTRY.get()
        + u64::try_from(code.len()).expect("fixed checkpoint guest length fits u64");

    checkpoint_emit_readback_request_setup(&mut code);
    code.extend_from_slice(&[0x66, 0xc7, 0x83, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);
    checkpoint_emit_debug(&mut code, VIRTIO_BLK_CHECKPOINT_READ_BARRIER);
    checkpoint_emit_second_completion_checks(&mut code);
    code.extend_from_slice(&[0x8a, 0x83, VIRTIO_ISR_OFFSET as u8, 0x02, 0x00, 0x00]);
    checkpoint_emit_cmp_al(&mut code, 1);
    checkpoint_emit_debug(&mut code, b'R');
    checkpoint_emit_debug(&mut code, b'D');
    code.push(0xf4);
    let terminal_rip = LONG_MODE_MMIO_GUEST_ENTRY.get()
        + u64::try_from(code.len()).expect("fixed checkpoint guest length fits u64");

    VirtioBlkCheckpointProgram {
        bytes: code,
        checkpoint_rip,
        terminal_rip,
    }
}

fn checkpoint_emit_write_request_setup(code: &mut Vec<u8>) {
    checkpoint_emit_request_descriptors(code, VIRTQ_DESC_F_NEXT);
    checkpoint_emit_request_header(code, VIRTIO_BLK_T_OUT);
    checkpoint_emit_status_sentinel(code);
    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_AVAIL_GPA);
    code.extend_from_slice(&[0xc7, 0x07, 0x00, 0x00, 0x01, 0x00]);
    code.extend_from_slice(&[0xc7, 0x47, 0x04, 0x00, 0x00, 0x00, 0x00]);
    checkpoint_emit_zero_used_ring(code);
}

fn checkpoint_emit_readback_request_setup(code: &mut Vec<u8>) {
    checkpoint_emit_request_descriptors(code, VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE);
    checkpoint_emit_request_header(code, VIRTIO_BLK_T_IN);
    checkpoint_emit_status_sentinel(code);

    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_DATA_GPA);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&0x5a5a_5a5a_5a5a_5a5a_u64.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xc7, 0xc1, 0x40, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xf3, 0x48, 0xab]);

    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_AVAIL_GPA);
    code.extend_from_slice(&[0xc7, 0x07, 0x00, 0x00, 0x02, 0x00]);
    code.extend_from_slice(&[0x66, 0xc7, 0x47, 0x06, 0x00, 0x00]);
}

fn checkpoint_emit_request_descriptors(code: &mut Vec<u8>, data_flags: u16) {
    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_DESCRIPTOR_GPA);
    code.extend_from_slice(&[0x48, 0xc7, 0x07]);
    code.extend_from_slice(&(VIRTIO_BLK_HEADER_GPA as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x08, 0x10, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xc7, 0x47, 0x0c]);
    let descriptor0_tail = u32::from(VIRTQ_DESC_F_NEXT) | (1_u32 << 16);
    code.extend_from_slice(&descriptor0_tail.to_le_bytes());

    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x10]);
    code.extend_from_slice(&(VIRTIO_BLK_DATA_GPA as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x18]);
    code.extend_from_slice(&(VIRTIO_BLK_SECTOR_SIZE as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x1c]);
    let descriptor1_tail = u32::from(data_flags) | (2_u32 << 16);
    code.extend_from_slice(&descriptor1_tail.to_le_bytes());

    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x20]);
    code.extend_from_slice(&(VIRTIO_BLK_STATUS_GPA as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x28, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xc7, 0x47, 0x2c]);
    code.extend_from_slice(&u32::from(VIRTQ_DESC_F_WRITE).to_le_bytes());
}

fn checkpoint_emit_request_header(code: &mut Vec<u8>, request_type: u32) {
    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_HEADER_GPA);
    code.extend_from_slice(&[0xc7, 0x07]);
    code.extend_from_slice(&request_type.to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x04, 0x00, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x08, 0x00, 0x00, 0x00, 0x00]);
}

fn checkpoint_emit_status_sentinel(code: &mut Vec<u8>) {
    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_STATUS_GPA);
    code.extend_from_slice(&[0xc6, 0x07, 0xff]);
}

fn checkpoint_emit_zero_used_ring(code: &mut Vec<u8>) {
    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_USED_GPA);
    code.extend_from_slice(&[0x48, 0xc7, 0x07, 0x00, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x08, 0x00, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x10, 0x00, 0x00, 0x00, 0x00]);
}

fn checkpoint_emit_first_completion_checks(code: &mut Vec<u8>) {
    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_USED_GPA);
    code.extend_from_slice(&[0x0f, 0xb7, 0x47, 0x02, 0x83, 0xf8, 0x01]);
    checkpoint_emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x04, 0x85, 0xc0]);
    checkpoint_emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x08]);
    checkpoint_emit_cmp_eax(code, 1);
    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_STATUS_GPA);
    code.extend_from_slice(&[0x8a, 0x07]);
    checkpoint_emit_cmp_al(code, VIRTIO_BLK_S_OK);
}

fn checkpoint_emit_second_completion_checks(code: &mut Vec<u8>) {
    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_USED_GPA);
    code.extend_from_slice(&[0x0f, 0xb7, 0x47, 0x02, 0x83, 0xf8, 0x02]);
    checkpoint_emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x0c, 0x85, 0xc0]);
    checkpoint_emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x10]);
    checkpoint_emit_cmp_eax(code, (VIRTIO_BLK_SECTOR_SIZE + 1) as u32);
    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_STATUS_GPA);
    code.extend_from_slice(&[0x8a, 0x07]);
    checkpoint_emit_cmp_al(code, VIRTIO_BLK_S_OK);

    checkpoint_emit_movabs(code, 7, VIRTIO_BLK_DATA_GPA);
    code.extend_from_slice(&[0x48, 0x8b, 0x07]);
    checkpoint_emit_movabs(code, 1, u64::from_le_bytes(*b"BLK-WRIT"));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    checkpoint_emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x48, 0x8b, 0x47, 0x08]);
    checkpoint_emit_movabs(code, 1, u64::from_le_bytes(*b"E-0000!!"));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    checkpoint_emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x48, 0x8b, 0x87, 0xf8, 0x01, 0x00, 0x00]);
    checkpoint_emit_movabs(code, 1, u64::from_le_bytes(*b"WRTBACK!"));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    checkpoint_emit_equal_or_ud2(code);
}

fn checkpoint_emit_cmp_eax(code: &mut Vec<u8>, expected: u32) {
    code.push(0x3d);
    code.extend_from_slice(&expected.to_le_bytes());
    checkpoint_emit_equal_or_ud2(code);
}

fn checkpoint_emit_cmp_al(code: &mut Vec<u8>, expected: u8) {
    code.extend_from_slice(&[0x3c, expected]);
    checkpoint_emit_equal_or_ud2(code);
}

fn checkpoint_emit_equal_or_ud2(code: &mut Vec<u8>) {
    code.extend_from_slice(&[0x74, 0x02, 0x0f, 0x0b]);
}

fn checkpoint_emit_debug(code: &mut Vec<u8>, byte: u8) {
    code.extend_from_slice(&[0xb0, byte, 0xe6, 0xe9]);
}

fn checkpoint_emit_movabs(code: &mut Vec<u8>, register: u8, value: u64) {
    debug_assert!(register < 8);
    code.extend_from_slice(&[0x48, 0xb8 + register]);
    code.extend_from_slice(&value.to_le_bytes());
}

fn checkpoint_emit_mmio_byte_write(code: &mut Vec<u8>, offset: u8, value: u8) {
    code.extend_from_slice(&[0xc6, 0x43, offset, value]);
}

fn checkpoint_emit_mmio_word_write(code: &mut Vec<u8>, offset: u8, value: u16) {
    code.extend_from_slice(&[0x66, 0xc7, 0x43, offset]);
    code.extend_from_slice(&value.to_le_bytes());
}

fn checkpoint_emit_mmio_dword_write(code: &mut Vec<u8>, offset: u8, value: u32) {
    code.extend_from_slice(&[0xc7, 0x43, offset]);
    code.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod virtio_blk_checkpoint_guest_tests {
    use super::*;

    #[test]
    fn queue_request_state_fits_one_owned_checkpoint_page() {
        let start = VIRTIO_BLK_CHECKPOINT_PAGE.get();
        let end = start + LONG_MODE_PAGE_SIZE;
        for address in [
            VIRTIO_BLK_DESCRIPTOR_GPA,
            VIRTIO_BLK_AVAIL_GPA,
            VIRTIO_BLK_USED_GPA,
            VIRTIO_BLK_HEADER_GPA,
            VIRTIO_BLK_DATA_GPA,
            VIRTIO_BLK_STATUS_GPA,
        ] {
            assert!(address >= start && address < end);
        }
        assert!(VIRTIO_BLK_DATA_GPA + VIRTIO_BLK_SECTOR_SIZE as u64 <= end);
    }

    #[test]
    fn guest_projects_version_1_into_selected_driver_feature_page() {
        assert_eq!((VIRTIO_F_VERSION_1 >> 32) as u32, 1);
        let program = build_virtio_blk_checkpoint_guest();
        let page1_driver_feature_write = [
            0xc7, 0x43, 0x08, 0x01, 0x00, 0x00, 0x00, // driver_feature_select = 1
            0xc7, 0x43, 0x0c, 0x01, 0x00, 0x00, 0x00, // page-local VERSION_1 bit
        ];
        assert!(program
            .bytes
            .windows(page1_driver_feature_write.len())
            .any(|window| window == page1_driver_feature_write));
    }

    #[test]
    fn guest_has_two_notify_writes_two_hlts_and_stable_proof_markers() {
        let program = build_virtio_blk_checkpoint_guest();
        assert_eq!(program.bytes.iter().filter(|byte| **byte == 0xf4).count(), 2);
        assert_eq!(
            program
                .bytes
                .windows(7)
                .filter(|window| *window == [0x66, 0xc7, 0x83, 0x00, 0x01, 0x00, 0x00])
                .count(),
            2
        );
        for marker in *b"BWONRD" {
            assert!(program
                .bytes
                .windows(4)
                .any(|window| window == [0xb0, marker, 0xe6, 0xe9]));
        }
        assert!(program.checkpoint_rip > LONG_MODE_MMIO_GUEST_ENTRY.get());
        assert!(program.terminal_rip > program.checkpoint_rip);
    }

    #[test]
    fn phase_exit_budgets_match_mmio_debug_and_hlt_contracts() {
        assert_eq!(VIRTIO_BLK_CHECKPOINT_FIRST_EXIT_BUDGET, 20 + 3 + 1);
        assert_eq!(VIRTIO_BLK_CHECKPOINT_REPLAY_EXIT_BUDGET, 2 + 3 + 1);
    }
}
