use crate::interrupt::{
    LONG_MODE_INTERRUPT_HANDLER, LONG_MODE_INTERRUPT_VECTOR, X86_RFLAGS_INTERRUPT_ENABLE,
};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use crate::memory::GuestMemory;
use crate::mmio::interrupt::LongModeMmioInterruptLayout;
use crate::mmio::{MmioBus, MmioDeviceEvent, MmioDeviceEventRecord};
use crate::portio::pci::virtio::{
    VIRTIO_F_VERSION_1, VIRTIO_ISR_OFFSET, VIRTIO_ISR_QUEUE_INTERRUPT, VIRTIO_STATUS_ACKNOWLEDGE,
    VIRTIO_STATUS_DRIVER, VIRTIO_STATUS_DRIVER_OK, VIRTIO_STATUS_FEATURES_OK,
};
use crate::portio::pci::virtio_blk::{
    deterministic_sector, VirtioBlkQueueCompletion, VIRTIO_BLK_CAPACITY_SECTORS,
    VIRTIO_BLK_SECTOR_SIZE, VIRTIO_BLK_S_OK, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE,
};
use crate::portio::virtio_blk_completion_interrupt_fixture::{
    VIRTIO_BLK_INTERRUPT_AVAIL_GPA, VIRTIO_BLK_INTERRUPT_BAR0_GPA,
    VIRTIO_BLK_INTERRUPT_DATA_GPA, VIRTIO_BLK_INTERRUPT_DESCRIPTOR_GPA,
    VIRTIO_BLK_INTERRUPT_HEADER_GPA, VIRTIO_BLK_INTERRUPT_STATUS_GPA,
    VIRTIO_BLK_INTERRUPT_USED_GPA,
};
use crate::portio::{PortIoBus, DEBUG_PORT};
use crate::vcpu::{MmioDirection, MmioExit, PortIoDirection, PortIoExit, VcpuExit};
use crate::vmexit::{dispatch_vcpu_exit, VmExitContinuation, VmExitDisposition};
use std::io;

pub const FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE: GuestPhysAddr =
    GuestPhysAddr::new(0x0001_8000);
pub const FULL_CONTROLLER_VIRTIO_BLK_CAPTURE_PROOF: &[u8; 1] = b"C";
pub const FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF: &[u8; 5] = b"NIARD";

const SETUP_EXIT_BUDGET: u32 = 20;
const REQUEST_EXIT_BUDGET: u32 = 8;
const QUEUE_SIZE: u16 = 4;
const CAPTURE_BYTE: u8 = b'C';
const NOTIFY_BYTE: u8 = b'N';
const HANDLER_BYTE: u8 = b'I';
const ACK_BYTE: u8 = b'A';
const READBACK_BYTE: u8 = b'R';
const DONE_BYTE: u8 = b'D';
const IOAPIC_CORRUPT_PIN: usize = 16;
const IOAPIC_REDIR_MASKED: u64 = 1 << 16;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullControllerVirtioBlkCheckpointGuestResult {
    capture_rip: u64,
    capture_rflags: u64,
    captured_avail_idx: u16,
    captured_used_idx: u16,
    mutation: BoundedFullControllerVirtioBlkCheckpointComparison,
    restored: BoundedFullControllerVirtioBlkCheckpointComparison,
    mutation_proof: Vec<u8>,
    replay_proof: Vec<u8>,
    mutation_assert_count: u32,
    mutation_deassert_count: u32,
    replay_assert_count: u32,
    replay_deassert_count: u32,
    replay_avail_idx: u16,
    replay_used_idx: u16,
    backing: Vec<u8>,
    readback: Vec<u8>,
    replay_rflags: u64,
}

impl FullControllerVirtioBlkCheckpointGuestResult {
    #[must_use]
    pub const fn capture_rip(&self) -> u64 {
        self.capture_rip
    }

    #[must_use]
    pub const fn capture_rflags(&self) -> u64 {
        self.capture_rflags
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
    pub const fn mutation(&self) -> &BoundedFullControllerVirtioBlkCheckpointComparison {
        &self.mutation
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedFullControllerVirtioBlkCheckpointComparison {
        &self.restored
    }

    #[must_use]
    pub fn mutation_proof(&self) -> &[u8] {
        &self.mutation_proof
    }

    #[must_use]
    pub fn replay_proof(&self) -> &[u8] {
        &self.replay_proof
    }

    #[must_use]
    pub const fn mutation_assert_count(&self) -> u32 {
        self.mutation_assert_count
    }

    #[must_use]
    pub const fn mutation_deassert_count(&self) -> u32 {
        self.mutation_deassert_count
    }

    #[must_use]
    pub const fn replay_assert_count(&self) -> u32 {
        self.replay_assert_count
    }

    #[must_use]
    pub const fn replay_deassert_count(&self) -> u32 {
        self.replay_deassert_count
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

    #[must_use]
    pub const fn replay_rflags(&self) -> u64 {
        self.replay_rflags
    }
}

struct CheckpointProgram {
    bytes: Vec<u8>,
    capture_rip: u64,
    request_quiescent_rip: u64,
}

#[derive(Debug)]
struct RequestPhase {
    proof: Vec<u8>,
    mmio_exits: Vec<MmioExit>,
    completion: VirtioBlkQueueCompletion,
    assert_count: u32,
    deassert_count: u32,
    rflags: u64,
}

pub fn run_full_controller_virtio_blk_checkpoint_guest(
) -> Result<FullControllerVirtioBlkCheckpointGuestResult, Error> {
    let program = build_program();
    let guest = FlatGuestImage::new(
        crate::mmio::long_mode::LONG_MODE_MMIO_GUEST_ENTRY,
        crate::mmio::long_mode::LONG_MODE_MMIO_GUEST_ENTRY,
        &program.bytes,
    )?;
    let handler_bytes = build_handler();
    let handler = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_HANDLER,
        LONG_MODE_INTERRUPT_HANDLER,
        &handler_bytes,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeMmioInterruptLayout::new(
        memory.region(),
        guest.entry(),
        crate::mmio::long_mode::LONG_MODE_MMIO_STACK_POINTER,
        LONG_MODE_INTERRUPT_VECTOR,
        handler.entry(),
    )
    .expect("fixed full-controller virtio-blk checkpoint layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(layout.interrupt_layout())?;
    let _ = vcpu.configure_legacy_pic_extint()?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty full-controller virtio-blk MSR policy is valid by construction");

    let mut mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA)
        .expect("fixed full-controller virtio-blk BAR does not overlap another device");

    let (capture_rip, capture_rflags) =
        run_setup_to_quiescent_capture(&mut vcpu, &mut mmio, program.capture_rip)?;
    require_device_setup(&mmio)?;
    if mmio.take_device_event_record().is_some() {
        return Err(checkpoint_error(
            "capture boundary has a pending device event before queue notification",
        ));
    }

    let checkpoint = BoundedFullControllerVirtioBlkCheckpoint::capture(
        &vcpu,
        &vm,
        &msr_policy,
        &mmio,
        VIRTIO_BLK_INTERRUPT_BAR0_GPA,
        &[FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE],
    )?;
    if checkpoint.device().checkpoint_last_avail_idx() != 0
        || checkpoint.device().checkpoint_last_used_idx() != 0
        || !checkpoint.device().checkpoint_quiescent()
    {
        return Err(checkpoint_error(
            "captured virtio-blk device is not the expected quiescent 0/0 queue state",
        ));
    }
    let captured_avail_idx = checkpoint.device().checkpoint_last_avail_idx();
    let captured_used_idx = checkpoint.device().checkpoint_last_used_idx();

    let mutation_phase = run_request_phase(
        &mut vcpu,
        &mut vm,
        &mut mmio,
        program.request_quiescent_rip,
        "mutation request",
    )?;
    require_request_memory(&vm, &mmio)?;
    corrupt_controller(checkpoint.controller(), &vcpu, &mut vm)?;

    let mutation = checkpoint.verify(&vcpu, &vm, &mmio)?;
    require_full_mismatch(&mutation)?;

    let restored = checkpoint.restore_and_verify(&vcpu, &mut vm, &mut mmio)?;
    require_full_exact(&restored)?;
    if mmio.take_device_event_record().is_some() {
        return Err(checkpoint_error(
            "restore left a pending virtio-blk device event",
        ));
    }
    let restored_device = mmio
        .capture_virtio_blk_checkpoint_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA)?
        .ok_or_else(|| checkpoint_error("restored virtio-blk device disappeared"))?;
    if restored_device.checkpoint_last_avail_idx() != 0
        || restored_device.checkpoint_last_used_idx() != 0
        || !restored_device.checkpoint_quiescent()
    {
        return Err(checkpoint_error(
            "restored virtio-blk queue did not return to captured 0/0 quiescent state",
        ));
    }

    let replay_phase = run_request_phase(
        &mut vcpu,
        &mut vm,
        &mut mmio,
        program.request_quiescent_rip,
        "replay request",
    )?;
    require_request_memory(&vm, &mmio)?;
    let replay_device = mmio
        .capture_virtio_blk_checkpoint_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA)?
        .ok_or_else(|| checkpoint_error("replayed virtio-blk device disappeared"))?;
    let replay_avail_idx = replay_device.checkpoint_last_avail_idx();
    let replay_used_idx = replay_device.checkpoint_last_used_idx();
    if replay_avail_idx != 1 || replay_used_idx != 1 {
        return Err(checkpoint_error(format!(
            "replayed queue did not advance restored state from 0/0 to 1/1: {replay_avail_idx}/{replay_used_idx}"
        )));
    }

    let backing = mmio
        .virtio_blk_sector_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA)
        .ok_or_else(|| checkpoint_error("replayed virtio-blk backing disappeared"))?
        .to_vec();
    let mut readback = vec![0_u8; VIRTIO_BLK_SECTOR_SIZE];
    vm.guest_memory()
        .expect("registered checkpoint memory remains VM-owned")
        .read(GuestPhysAddr::new(VIRTIO_BLK_INTERRUPT_DATA_GPA), &mut readback)?;
    if backing.as_slice() != deterministic_sector() || readback.as_slice() != deterministic_sector() {
        return Err(checkpoint_error(
            "replayed virtio-blk data path did not preserve deterministic backing/readback",
        ));
    }

    Ok(FullControllerVirtioBlkCheckpointGuestResult {
        capture_rip,
        capture_rflags,
        captured_avail_idx,
        captured_used_idx,
        mutation,
        restored,
        mutation_proof: mutation_phase.proof,
        replay_proof: replay_phase.proof,
        mutation_assert_count: mutation_phase.assert_count,
        mutation_deassert_count: mutation_phase.deassert_count,
        replay_assert_count: replay_phase.assert_count,
        replay_deassert_count: replay_phase.deassert_count,
        replay_avail_idx,
        replay_used_idx,
        backing,
        readback,
        replay_rflags: replay_phase.rflags,
    })
}

fn run_setup_to_quiescent_capture(
    vcpu: &mut crate::vcpu::Vcpu,
    mmio: &mut MmioBus,
    expected_rip: u64,
) -> Result<(u64, u64), Error> {
    let mut port_io = PortIoBus::with_debug_port();
    let mut exits = 0_u32;
    loop {
        if exits >= SETUP_EXIT_BUDGET {
            return Err(checkpoint_error(
                "setup exceeded exact exit budget before capture arm marker",
            ));
        }
        let exit = vcpu.run_once()?;
        exits += 1;
        let disposition = dispatch_vcpu_exit(vcpu, exit, &mut port_io, mmio)?;
        match disposition {
            VmExitDisposition::Continue(continuation) => {
                if is_debug_output(&continuation, CAPTURE_BYTE) {
                    break;
                }
            }
            VmExitDisposition::Stopped(report) => {
                return Err(checkpoint_error(format!(
                    "unexpected terminal exit during checkpoint setup: {report}"
                )));
            }
        }
    }
    if exits != SETUP_EXIT_BUDGET {
        return Err(checkpoint_error(format!(
            "expected exactly {SETUP_EXIT_BUDGET} setup exits, got {exits}"
        )));
    }
    if port_io.debug_output() != Some(FULL_CONTROLLER_VIRTIO_BLK_CAPTURE_PROOF) {
        return Err(checkpoint_error(format!(
            "unexpected capture proof: {:?}",
            port_io.debug_output()
        )));
    }
    run_single_step_to_quiescence(vcpu, expected_rip, "capture quiescent boundary")
}

fn run_request_phase(
    vcpu: &mut crate::vcpu::Vcpu,
    vm: &mut crate::kvm::Vm,
    mmio: &mut MmioBus,
    expected_rip: u64,
    stage: &'static str,
) -> Result<RequestPhase, Error> {
    let mut port_io = PortIoBus::with_debug_port();
    let mut mmio_exits = Vec::new();
    let mut completion = None;
    let mut line_asserted = false;
    let mut assert_count = 0_u32;
    let mut deassert_count = 0_u32;
    let mut exits = 0_u32;

    loop {
        if exits >= REQUEST_EXIT_BUDGET {
            return Err(checkpoint_error(format!(
                "{stage} exceeded exact exit budget before completion barrier"
            )));
        }
        let exit = vcpu.run_once()?;
        exits += 1;
        let disposition = dispatch_vcpu_exit(vcpu, exit, &mut port_io, mmio)?;
        match disposition {
            VmExitDisposition::Continue(continuation) => {
                if is_debug_output(&continuation, NOTIFY_BYTE) {
                    if completion.is_some() || line_asserted {
                        return Err(checkpoint_error(format!(
                            "{stage} observed duplicate notify barrier"
                        )));
                    }
                    let event = mmio.take_device_event_record().ok_or_else(|| {
                        checkpoint_error(format!("{stage} notify has no pending device event"))
                    })?;
                    let expected = MmioDeviceEventRecord::new(
                        VIRTIO_BLK_INTERRUPT_BAR0_GPA,
                        MmioDeviceEvent::VirtioQueueNotified { queue: 0 },
                    );
                    if event != expected {
                        return Err(checkpoint_error(format!(
                            "{stage} observed unexpected device event: {event:?}"
                        )));
                    }
                    let memory = vm.guest_memory_mut().ok_or_else(|| {
                        checkpoint_error(format!("{stage} VM lost registered guest memory"))
                    })?;
                    let observed = mmio
                        .process_virtio_blk_notification(VIRTIO_BLK_INTERRUPT_BAR0_GPA, memory)
                        .map_err(|error| {
                            checkpoint_error(format!("{stage} queue processing failed: {error}"))
                        })?
                        .ok_or_else(|| checkpoint_error(format!("{stage} virtio BAR disappeared")))?;
                    vm.set_gsi_level(KvmBackend::IRQCHIP_GSI, true)?;
                    completion = Some(observed);
                    line_asserted = true;
                    assert_count += 1;
                } else if is_debug_output(&continuation, ACK_BYTE) {
                    if completion.is_none() || !line_asserted {
                        return Err(checkpoint_error(format!(
                            "{stage} ISR ACK arrived without asserted completion line"
                        )));
                    }
                    vm.set_gsi_level(KvmBackend::IRQCHIP_GSI, false)?;
                    line_asserted = false;
                    deassert_count += 1;
                }

                let done = is_debug_output(&continuation, DONE_BYTE);
                if let VmExitContinuation::Mmio(access) = continuation {
                    mmio_exits.push(access);
                }
                if done {
                    break;
                }
            }
            VmExitDisposition::Stopped(report) => {
                return Err(checkpoint_error(format!(
                    "{stage} terminated before completion barrier: {report}"
                )));
            }
        }
    }

    if exits != REQUEST_EXIT_BUDGET {
        return Err(checkpoint_error(format!(
            "expected exactly {REQUEST_EXIT_BUDGET} exits for {stage}, got {exits}"
        )));
    }
    if line_asserted || assert_count != 1 || deassert_count != 1 {
        return Err(checkpoint_error(format!(
            "{stage} line lifecycle mismatch: asserted={line_asserted} assert={assert_count} deassert={deassert_count}"
        )));
    }
    if mmio.take_device_event_record().is_some() {
        return Err(checkpoint_error(format!(
            "{stage} left an unexpected device event"
        )));
    }
    validate_request_mmio(&mmio_exits, stage)?;
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF {
        return Err(checkpoint_error(format!(
            "{stage} proof mismatch: expected {:?}, got {proof:?}",
            FULL_CONTROLLER_VIRTIO_BLK_REQUEST_PROOF
        )));
    }
    let completion = completion.ok_or_else(|| checkpoint_error(format!("{stage} never completed")))?;
    if completion.descriptor_id() != 0
        || completion.length() != (VIRTIO_BLK_SECTOR_SIZE + 1) as u32
        || completion.sector() != 0
    {
        return Err(checkpoint_error(format!(
            "{stage} completion mismatch: {completion:?}"
        )));
    }
    let (_, rflags) = run_single_step_to_quiescence(vcpu, expected_rip, stage)?;

    Ok(RequestPhase {
        proof,
        mmio_exits,
        completion,
        assert_count,
        deassert_count,
        rflags,
    })
}

fn run_single_step_to_quiescence(
    vcpu: &mut crate::vcpu::Vcpu,
    expected_rip: u64,
    stage: &'static str,
) -> Result<(u64, u64), Error> {
    vcpu.set_guest_single_step(true)?;
    let exit_result = vcpu.run_once();
    let disable_result = vcpu.set_guest_single_step(false);
    let exit = match (exit_result, disable_result) {
        (Ok(exit), Ok(())) => exit,
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
    };
    if exit != VcpuExit::Debug {
        return Err(checkpoint_error(format!(
            "{stage}: expected KVM_EXIT_DEBUG {}, got {}",
            VcpuExit::Debug.reason(),
            exit.reason()
        )));
    }
    let registers = vcpu.registers()?;
    if registers.rip != expected_rip
        || registers.rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || registers.rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
    {
        return Err(checkpoint_error(format!(
            "{stage}: expected IF-set quiescent debug at rip={expected_rip:#x}, got rip={:#x}, rflags={:#x}",
            registers.rip, registers.rflags
        )));
    }
    Ok((registers.rip, registers.rflags))
}

fn require_device_setup(mmio: &MmioBus) -> Result<(), Error> {
    let expected_status = VIRTIO_STATUS_ACKNOWLEDGE
        | VIRTIO_STATUS_DRIVER
        | VIRTIO_STATUS_FEATURES_OK
        | VIRTIO_STATUS_DRIVER_OK;
    if mmio.virtio_blk_status_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA) != Some(expected_status)
        || mmio.virtio_blk_driver_features_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA)
            != Some(VIRTIO_F_VERSION_1)
        || mmio.virtio_blk_queue_enabled_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA) != Some(true)
    {
        return Err(checkpoint_error(
            "virtio-blk setup was not fully negotiated and queue-enabled before capture",
        ));
    }
    Ok(())
}

fn require_request_memory(vm: &crate::kvm::Vm, mmio: &MmioBus) -> Result<(), Error> {
    let memory = vm
        .guest_memory()
        .ok_or_else(|| checkpoint_error("VM lost registered memory during request verification"))?;
    let used_idx = read_u16(memory, VIRTIO_BLK_INTERRUPT_USED_GPA + 2)?;
    let used_id = read_u32(memory, VIRTIO_BLK_INTERRUPT_USED_GPA + 4)?;
    let used_len = read_u32(memory, VIRTIO_BLK_INTERRUPT_USED_GPA + 8)?;
    let mut data = vec![0_u8; VIRTIO_BLK_SECTOR_SIZE];
    memory.read(GuestPhysAddr::new(VIRTIO_BLK_INTERRUPT_DATA_GPA), &mut data)?;
    let mut status = [0xff_u8];
    memory.read(GuestPhysAddr::new(VIRTIO_BLK_INTERRUPT_STATUS_GPA), &mut status)?;
    let device = mmio
        .capture_virtio_blk_checkpoint_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA)?
        .ok_or_else(|| checkpoint_error("virtio-blk device disappeared during request verification"))?;
    if used_idx != 1
        || used_id != 0
        || used_len != (VIRTIO_BLK_SECTOR_SIZE + 1) as u32
        || data.as_slice() != deterministic_sector()
        || status[0] != VIRTIO_BLK_S_OK
        || device.checkpoint_last_avail_idx() != 1
        || device.checkpoint_last_used_idx() != 1
        || !device.checkpoint_quiescent()
    {
        return Err(checkpoint_error(format!(
            "request verification mismatch: used=({used_idx},{used_id},{used_len}) status={:#x} queue={}/{}",
            status[0],
            device.checkpoint_last_avail_idx(),
            device.checkpoint_last_used_idx()
        )));
    }
    Ok(())
}

fn corrupt_controller(
    checkpoint: &BoundedFullControllerCheckpoint,
    vcpu: &crate::vcpu::Vcpu,
    vm: &mut crate::kvm::Vm,
) -> Result<(), Error> {
    vm.restore_master_pic_state(&checkpoint.master_pic().with_imr(checkpoint.master_pic().imr() ^ 1))?;
    vm.restore_slave_pic_state(&checkpoint.slave_pic().with_imr(checkpoint.slave_pic().imr() ^ 1))?;
    let pin = checkpoint
        .ioapic()
        .redirection_entry(IOAPIC_CORRUPT_PIN)
        .ok_or_else(|| checkpoint_error("fixed IOAPIC corruption pin is out of range"))?;
    let ioapic = checkpoint
        .ioapic()
        .with_redirection_entry(IOAPIC_CORRUPT_PIN, pin ^ IOAPIC_REDIR_MASKED)
        .ok_or_else(|| checkpoint_error("failed to build IOAPIC corruption snapshot"))?;
    vm.restore_ioapic_state(&ioapic)?;

    let mut lapic = checkpoint.lapic().clone();
    let lint0 = controller_read_lapic_register(&lapic, APIC_LVT0_OFFSET);
    controller_write_lapic_register(&mut lapic, APIC_LVT0_OFFSET, lint0 ^ APIC_LVT_MASKED);
    vcpu.restore_lapic_checkpoint_state(&lapic)?;
    Ok(())
}

fn require_full_mismatch(
    comparison: &BoundedFullControllerVirtioBlkCheckpointComparison,
) -> Result<(), Error> {
    let controller = comparison.controller();
    if controller.page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE) != Some(false)
        || controller.vcpu_exact()
        || controller.master_pic_exact()
        || controller.slave_pic_exact()
        || controller.ioapic_exact()
        || controller.lapic_exact()
        || comparison.device_exact()
    {
        return Err(checkpoint_error(format!(
            "expected full page/vcpu/controller/device mismatch, got page={:?} vcpu={} master={} slave={} ioapic={} lapic={} device={}",
            controller.page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE),
            controller.vcpu_exact(),
            controller.master_pic_exact(),
            controller.slave_pic_exact(),
            controller.ioapic_exact(),
            controller.lapic_exact(),
            comparison.device_exact()
        )));
    }
    Ok(())
}

fn require_full_exact(
    comparison: &BoundedFullControllerVirtioBlkCheckpointComparison,
) -> Result<(), Error> {
    let controller = comparison.controller();
    if !comparison.is_exact_match()
        || controller.page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE) != Some(true)
        || !controller.vcpu_exact()
        || !controller.master_pic_exact()
        || !controller.slave_pic_exact()
        || !controller.ioapic_exact()
        || !controller.lapic_exact()
        || !comparison.device_exact()
    {
        return Err(checkpoint_error(format!(
            "combined restore was not exact: page={:?} vcpu={} master={} slave={} ioapic={} lapic={} device={}",
            controller.page_exact(FULL_CONTROLLER_VIRTIO_BLK_CHECKPOINT_PAGE),
            controller.vcpu_exact(),
            controller.master_pic_exact(),
            controller.slave_pic_exact(),
            controller.ioapic_exact(),
            controller.lapic_exact(),
            comparison.device_exact()
        )));
    }
    Ok(())
}

fn validate_request_mmio(exits: &[MmioExit], stage: &'static str) -> Result<(), Error> {
    let expected = [
        (0x100_u64, MmioDirection::Write, 2_u32),
        (VIRTIO_ISR_OFFSET, MmioDirection::Read, 1_u32),
        (VIRTIO_ISR_OFFSET, MmioDirection::Read, 1_u32),
    ];
    if exits.len() != expected.len() {
        return Err(checkpoint_error(format!(
            "{stage}: expected {} request MMIO exits, got {}",
            expected.len(),
            exits.len()
        )));
    }
    for (exit, (offset, direction, length)) in exits.iter().zip(expected) {
        if exit.address() != VIRTIO_BLK_INTERRUPT_BAR0_GPA + offset
            || exit.direction() != direction
            || exit.length() != length
        {
            return Err(checkpoint_error(format!(
                "{stage}: unexpected MMIO exit {exit:?}"
            )));
        }
    }
    if exits[0].write_data() != 0_u16.to_le_bytes() {
        return Err(checkpoint_error(format!(
            "{stage}: notify payload was not queue 0"
        )));
    }
    Ok(())
}

fn is_debug_output(continuation: &VmExitContinuation, expected: u8) -> bool {
    matches!(
        continuation,
        VmExitContinuation::PortIo(io)
            if io.direction() == PortIoDirection::Out
                && io.port() == DEBUG_PORT
                && io.size() == 1
                && io.count() == 1
                && io.output_data() == [expected]
    )
}

fn read_u16(memory: &GuestMemory, address: u64) -> Result<u16, Error> {
    let mut bytes = [0_u8; 2];
    memory.read(GuestPhysAddr::new(address), &mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(memory: &GuestMemory, address: u64) -> Result<u32, Error> {
    let mut bytes = [0_u8; 4];
    memory.read(GuestPhysAddr::new(address), &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn checkpoint_error(detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation: "full-controller virtio-blk checkpoint proof",
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

fn build_program() -> CheckpointProgram {
    let mut code = Vec::new();
    emit_pic_setup(&mut code);
    code.extend_from_slice(&[0xfb, 0x90]);
    emit_movabs(&mut code, 3, 0x0050_0000);
    code.extend_from_slice(&[0x48, 0x8b, 0x83, 0x00, 0x03, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0x83, 0xf8, VIRTIO_BLK_CAPACITY_SECTORS as u8]);
    emit_equal_or_ud2(&mut code);
    emit_mmio_byte_write(&mut code, 0x14, VIRTIO_STATUS_ACKNOWLEDGE);
    emit_mmio_byte_write(
        &mut code,
        0x14,
        VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER,
    );
    emit_mmio_dword_write(&mut code, 0x00, 1);
    code.extend_from_slice(&[0x8b, 0x43, 0x04]);
    emit_cmp_eax(&mut code, 1);
    emit_mmio_dword_write(&mut code, 0x08, 1);
    emit_mmio_dword_write(&mut code, 0x0c, 1);
    emit_mmio_byte_write(
        &mut code,
        0x14,
        VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK,
    );
    emit_mmio_word_write(&mut code, 0x16, 0);
    emit_mmio_word_write(&mut code, 0x18, QUEUE_SIZE);
    emit_mmio_dword_write(&mut code, 0x20, VIRTIO_BLK_INTERRUPT_DESCRIPTOR_GPA as u32);
    emit_mmio_dword_write(&mut code, 0x24, 0);
    emit_mmio_dword_write(&mut code, 0x28, VIRTIO_BLK_INTERRUPT_AVAIL_GPA as u32);
    emit_mmio_dword_write(&mut code, 0x2c, 0);
    emit_mmio_dword_write(&mut code, 0x30, VIRTIO_BLK_INTERRUPT_USED_GPA as u32);
    emit_mmio_dword_write(&mut code, 0x34, 0);
    emit_mmio_word_write(&mut code, 0x1c, 1);
    emit_mmio_byte_write(
        &mut code,
        0x14,
        VIRTIO_STATUS_ACKNOWLEDGE
            | VIRTIO_STATUS_DRIVER
            | VIRTIO_STATUS_FEATURES_OK
            | VIRTIO_STATUS_DRIVER_OK,
    );
    code.extend_from_slice(&[0x8a, 0x43, 0x14]);
    emit_cmp_al(
        &mut code,
        VIRTIO_STATUS_ACKNOWLEDGE
            | VIRTIO_STATUS_DRIVER
            | VIRTIO_STATUS_FEATURES_OK
            | VIRTIO_STATUS_DRIVER_OK,
    );
    emit_ring_setup(&mut code);
    emit_debug(&mut code, CAPTURE_BYTE);
    code.push(0x90);
    let capture_rip = crate::mmio::long_mode::LONG_MODE_MMIO_GUEST_ENTRY.get() + code.len() as u64;

    code.extend_from_slice(&[0x66, 0xc7, 0x83, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);
    emit_debug(&mut code, NOTIFY_BYTE);
    emit_guest_completion_checks(&mut code);
    emit_movabs(&mut code, 3, 0x0050_0000);
    code.extend_from_slice(&[0x8a, 0x83]);
    code.extend_from_slice(&(VIRTIO_ISR_OFFSET as u32).to_le_bytes());
    emit_cmp_al(&mut code, 0);
    emit_debug(&mut code, READBACK_BYTE);
    emit_debug(&mut code, DONE_BYTE);
    code.push(0x90);
    let request_quiescent_rip =
        crate::mmio::long_mode::LONG_MODE_MMIO_GUEST_ENTRY.get() + code.len() as u64;
    code.push(0xf4);

    CheckpointProgram {
        bytes: code,
        capture_rip,
        request_quiescent_rip,
    }
}

fn build_handler() -> Vec<u8> {
    let mut code = Vec::new();
    emit_debug(&mut code, HANDLER_BYTE);
    emit_movabs(&mut code, 3, 0x0050_0000);
    code.extend_from_slice(&[0x8a, 0x83]);
    code.extend_from_slice(&(VIRTIO_ISR_OFFSET as u32).to_le_bytes());
    emit_cmp_al(&mut code, VIRTIO_ISR_QUEUE_INTERRUPT);
    emit_debug(&mut code, ACK_BYTE);
    code.extend_from_slice(&[0xb0, 0x20, 0xe6, 0x20]);
    code.extend_from_slice(&[0x48, 0xcf]);
    code
}

fn emit_pic_setup(code: &mut Vec<u8>) {
    code.push(0xfa);
    code.extend_from_slice(&[0xb0, 0x11, 0xe6, 0x20, 0xe6, 0xa0]);
    code.extend_from_slice(&[0xb0, 0x40, 0xe6, 0x21]);
    code.extend_from_slice(&[0xb0, 0x48, 0xe6, 0xa1]);
    code.extend_from_slice(&[0xb0, 0x04, 0xe6, 0x21]);
    code.extend_from_slice(&[0xb0, 0x02, 0xe6, 0xa1]);
    code.extend_from_slice(&[0xb0, 0x01, 0xe6, 0x21, 0xe6, 0xa1]);
    code.extend_from_slice(&[0xb0, 0xfe, 0xe6, 0x21]);
    code.extend_from_slice(&[0xb0, 0xff, 0xe6, 0xa1]);
}

fn emit_cmp_eax(code: &mut Vec<u8>, expected: u32) {
    code.push(0x3d);
    code.extend_from_slice(&expected.to_le_bytes());
    emit_equal_or_ud2(code);
}

fn emit_cmp_al(code: &mut Vec<u8>, expected: u8) {
    code.extend_from_slice(&[0x3c, expected]);
    emit_equal_or_ud2(code);
}

fn emit_equal_or_ud2(code: &mut Vec<u8>) {
    code.extend_from_slice(&[0x74, 0x02, 0x0f, 0x0b]);
}

fn emit_debug(code: &mut Vec<u8>, byte: u8) {
    code.extend_from_slice(&[0xb0, byte, 0xe6, 0xe9]);
}

fn emit_movabs(code: &mut Vec<u8>, register: u8, value: u64) {
    debug_assert!(register < 8);
    code.extend_from_slice(&[0x48, 0xb8 + register]);
    code.extend_from_slice(&value.to_le_bytes());
}

fn emit_mmio_byte_write(code: &mut Vec<u8>, offset: u8, value: u8) {
    code.extend_from_slice(&[0xc6, 0x43, offset, value]);
}

fn emit_mmio_word_write(code: &mut Vec<u8>, offset: u8, value: u16) {
    code.extend_from_slice(&[0x66, 0xc7, 0x43, offset]);
    code.extend_from_slice(&value.to_le_bytes());
}

fn emit_mmio_dword_write(code: &mut Vec<u8>, offset: u8, value: u32) {
    code.extend_from_slice(&[0xc7, 0x43, offset]);
    code.extend_from_slice(&value.to_le_bytes());
}

fn emit_ring_setup(code: &mut Vec<u8>) {
    emit_movabs(code, 7, VIRTIO_BLK_INTERRUPT_DESCRIPTOR_GPA);
    code.extend_from_slice(&[0x48, 0xc7, 0x07]);
    code.extend_from_slice(&(VIRTIO_BLK_INTERRUPT_HEADER_GPA as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x08, 0x10, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xc7, 0x47, 0x0c]);
    let descriptor0_tail = u32::from(VIRTQ_DESC_F_NEXT) | (1_u32 << 16);
    code.extend_from_slice(&descriptor0_tail.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x10]);
    code.extend_from_slice(&(VIRTIO_BLK_INTERRUPT_DATA_GPA as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x18]);
    code.extend_from_slice(&(VIRTIO_BLK_SECTOR_SIZE as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x1c]);
    let descriptor1_tail = u32::from(VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE) | (2_u32 << 16);
    code.extend_from_slice(&descriptor1_tail.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x20]);
    code.extend_from_slice(&(VIRTIO_BLK_INTERRUPT_STATUS_GPA as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x28, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xc7, 0x47, 0x2c]);
    let descriptor2_tail = u32::from(VIRTQ_DESC_F_WRITE);
    code.extend_from_slice(&descriptor2_tail.to_le_bytes());

    emit_movabs(code, 7, VIRTIO_BLK_INTERRUPT_HEADER_GPA);
    code.extend_from_slice(&[0x48, 0xc7, 0x07, 0, 0, 0, 0]);
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x08, 0, 0, 0, 0]);
    emit_movabs(code, 7, VIRTIO_BLK_INTERRUPT_STATUS_GPA);
    code.extend_from_slice(&[0xc6, 0x07, 0xff]);
    emit_movabs(code, 7, VIRTIO_BLK_INTERRUPT_AVAIL_GPA);
    code.extend_from_slice(&[0xc7, 0x07, 0, 0, 1, 0]);
    code.extend_from_slice(&[0xc7, 0x47, 0x04, 0, 0, 0, 0]);
    emit_movabs(code, 7, VIRTIO_BLK_INTERRUPT_USED_GPA);
    code.extend_from_slice(&[0x48, 0xc7, 0x07, 0, 0, 0, 0]);
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x08, 0, 0, 0, 0]);
}

fn emit_guest_completion_checks(code: &mut Vec<u8>) {
    emit_movabs(code, 7, VIRTIO_BLK_INTERRUPT_USED_GPA);
    code.extend_from_slice(&[0x0f, 0xb7, 0x47, 0x02, 0x83, 0xf8, 0x01]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x04, 0x85, 0xc0]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x08]);
    emit_cmp_eax(code, (VIRTIO_BLK_SECTOR_SIZE + 1) as u32);
    emit_movabs(code, 7, VIRTIO_BLK_INTERRUPT_STATUS_GPA);
    code.extend_from_slice(&[0x8a, 0x07]);
    emit_cmp_al(code, VIRTIO_BLK_S_OK);
    emit_movabs(code, 7, VIRTIO_BLK_INTERRUPT_DATA_GPA);
    code.extend_from_slice(&[0x48, 0x8b, 0x07]);
    emit_movabs(code, 1, u64::from_le_bytes(*b"BLK-SECT"));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x48, 0x8b, 0x87, 0xf8, 0x01, 0x00, 0x00]);
    emit_movabs(code, 1, u64::from_le_bytes(*b"BLKEND!!"));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    emit_equal_or_ud2(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_has_two_nonserviceable_debug_boundaries_and_intx_handler() {
        let program = build_program();
        assert!(program.bytes.ends_with(&[0x90, 0xf4]));
        assert!(program.capture_rip < program.request_quiescent_rip);
        for marker in *b"CNRD" {
            assert!(program
                .bytes
                .windows(4)
                .any(|window| window == [0xb0, marker, 0xe6, 0xe9]));
        }
        let handler = build_handler();
        for marker in *b"IA" {
            assert!(handler
                .windows(4)
                .any(|window| window == [0xb0, marker, 0xe6, 0xe9]));
        }
        assert!(handler.ends_with(&[0x48, 0xcf]));
    }

    #[test]
    fn exact_exit_budgets_match_setup_and_request_contracts() {
        assert_eq!(SETUP_EXIT_BUDGET, 19 + 1);
        assert_eq!(REQUEST_EXIT_BUDGET, 3 + 5);
    }
}
