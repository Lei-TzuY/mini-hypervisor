use super::{
    BoundedFullControllerTwoVirtioBlkCheckpoint,
    BoundedFullControllerTwoVirtioBlkCheckpointComparison,
    VersionedTwoDeviceCheckpointTransactionV1,
};
use crate::error::{Error, HostEnvironmentError, VmExitError};
use crate::interrupt::{
    LongModeInterruptGate, LongModeInterruptLayout, X86_RFLAGS_INTERRUPT_ENABLE,
};
use crate::kvm::msr::GuestMsrAccessPolicy;
use crate::kvm::sys::{
    default_two_host_registration_pair, HostRegistrationPairCheckpoint,
    ReconstructedHostRegistrationPair, TWO_HOST_REGISTRATION_FIRST_VECTOR,
    TWO_HOST_REGISTRATION_SECOND_VECTOR,
};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{LongModeBootLayout, LONG_MODE_IDENTITY_MAP_SIZE, LONG_MODE_PAGE_SIZE};
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::mmio::long_mode::{
    LongModeMmioBootLayout, LongModeMmioPageMapping, LONG_MODE_MMIO_STACK_POINTER,
    LONG_MODE_MMIO_VIRTUAL_PAGE,
};
use crate::mmio::multi_device::MULTI_DEVICE_SECOND_VIRTUAL_PAGE;
use crate::mmio::MmioBus;
use crate::portio::pci::virtio::{
    VIRTIO_F_VERSION_1, VIRTIO_ISR_OFFSET, VIRTIO_ISR_QUEUE_INTERRUPT, VIRTIO_STATUS_ACKNOWLEDGE,
    VIRTIO_STATUS_DRIVER, VIRTIO_STATUS_DRIVER_OK, VIRTIO_STATUS_FEATURES_OK,
};
use crate::portio::pci::virtio_blk::{
    deterministic_sector, VirtioBlkDevice, VirtioBlkQueueCompletion, VIRTIO_BLK_SECTOR_SIZE,
    VIRTIO_BLK_S_OK, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE,
};
use crate::portio::{PortIoBus, DEBUG_PORT};
use crate::vcpu::{PortIoDirection, Vcpu, VcpuExit, VcpuId};
use crate::vmexit::{dispatch_vcpu_exit, VmExitContinuation, VmExitDisposition};

pub const TRANSACTION_COUPLED_FIRST_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x0001_8000);
pub const TRANSACTION_COUPLED_SECOND_PAGE: GuestPhysAddr = GuestPhysAddr::new(0x0001_9000);
pub const TRANSACTION_COUPLED_CAPTURE_PROOF: &[u8; 1] = b"C";
pub const TRANSACTION_COUPLED_REPLAY_PROOF: &[u8; 9] = b"A0aMB1bND";

const ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x0001_0000);
const FIRST_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x0001_1000);
const SECOND_HANDLER: GuestPhysAddr = GuestPhysAddr::new(0x0001_2000);
const CORRUPT_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x0001_4000);
const CORRUPT_STACK: u64 = 0x001f_dff8;
const QUEUE_SIZE: u16 = 4;
const WAIT_MILLIS: i32 = 5_000;
const REPLAY_EXIT_BUDGET: u32 = 24;
const CAPTURE_MARKER: u8 = b'C';
const FIRST_NOTIFY_MARKER: u8 = b'A';
const FIRST_HANDLER_MARKER: u8 = b'0';
const FIRST_ACK_MARKER: u8 = b'a';
const FIRST_RESUMED_MARKER: u8 = b'M';
const SECOND_NOTIFY_MARKER: u8 = b'B';
const SECOND_HANDLER_MARKER: u8 = b'1';
const SECOND_ACK_MARKER: u8 = b'b';
const SECOND_RESUMED_MARKER: u8 = b'N';
const DONE_MARKER: u8 = b'D';
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

const FIRST_DESC: u64 = 0x0001_8000;
const FIRST_AVAIL: u64 = 0x0001_8100;
const FIRST_USED: u64 = 0x0001_8200;
const FIRST_HEADER: u64 = 0x0001_8300;
const FIRST_DATA: u64 = 0x0001_8400;
const FIRST_STATUS: u64 = 0x0001_8600;

const SECOND_DESC: u64 = 0x0001_9000;
const SECOND_AVAIL: u64 = 0x0001_9100;
const SECOND_USED: u64 = 0x0001_9200;
const SECOND_HEADER: u64 = 0x0001_9300;
const SECOND_DATA: u64 = 0x0001_9400;
const SECOND_STATUS: u64 = 0x0001_9600;

#[derive(Debug, Clone, Copy)]
struct QueueLayout {
    desc: u64,
    avail: u64,
    used: u64,
    header: u64,
    data: u64,
    status: u64,
}

const FIRST_QUEUE: QueueLayout = QueueLayout {
    desc: FIRST_DESC,
    avail: FIRST_AVAIL,
    used: FIRST_USED,
    header: FIRST_HEADER,
    data: FIRST_DATA,
    status: FIRST_STATUS,
};
const SECOND_QUEUE: QueueLayout = QueueLayout {
    desc: SECOND_DESC,
    avail: SECOND_AVAIL,
    used: SECOND_USED,
    header: SECOND_HEADER,
    data: SECOND_DATA,
    status: SECOND_STATUS,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionCoupledDualDeviceReplayResult {
    mutation: BoundedFullControllerTwoVirtioBlkCheckpointComparison,
    restored: BoundedFullControllerTwoVirtioBlkCheckpointComparison,
    transaction_version: u16,
    encoded_len: usize,
    checkpoint_encoded_len: usize,
    registration_pair_encoded_len: usize,
    bars: [u64; 2],
    replay_queue_indices: [[u16; 2]; 2],
    doorbell_events: [u64; 2],
    irqfd_signals: [u32; 2],
    readback: [Vec<u8>; 2],
    backing: [Vec<u8>; 2],
    proof: Vec<u8>,
    completion_rflags: u64,
}

impl TransactionCoupledDualDeviceReplayResult {
    #[must_use]
    pub const fn mutation(&self) -> &BoundedFullControllerTwoVirtioBlkCheckpointComparison {
        &self.mutation
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedFullControllerTwoVirtioBlkCheckpointComparison {
        &self.restored
    }

    #[must_use]
    pub const fn transaction_version(&self) -> u16 {
        self.transaction_version
    }

    #[must_use]
    pub const fn encoded_len(&self) -> usize {
        self.encoded_len
    }

    #[must_use]
    pub const fn checkpoint_encoded_len(&self) -> usize {
        self.checkpoint_encoded_len
    }

    #[must_use]
    pub const fn registration_pair_encoded_len(&self) -> usize {
        self.registration_pair_encoded_len
    }

    #[must_use]
    pub const fn bars(&self) -> [u64; 2] {
        self.bars
    }

    #[must_use]
    pub const fn replay_queue_indices(&self) -> [[u16; 2]; 2] {
        self.replay_queue_indices
    }

    #[must_use]
    pub const fn doorbell_events(&self) -> [u64; 2] {
        self.doorbell_events
    }

    #[must_use]
    pub const fn irqfd_signals(&self) -> [u32; 2] {
        self.irqfd_signals
    }

    #[must_use]
    pub fn readback(&self) -> [&[u8]; 2] {
        [&self.readback[0], &self.readback[1]]
    }

    #[must_use]
    pub fn backing(&self) -> [&[u8]; 2] {
        [&self.backing[0], &self.backing[1]]
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn completion_rflags(&self) -> u64 {
        self.completion_rflags
    }
}

pub fn run_transaction_coupled_dual_device_replay_guest(
) -> Result<TransactionCoupledDualDeviceReplayResult, Error> {
    let program = build_guest_program();
    let guest = FlatGuestImage::new(ENTRY, ENTRY, &program.bytes)?;
    let first_handler_bytes = build_handler(
        LONG_MODE_MMIO_VIRTUAL_PAGE,
        FIRST_HANDLER_MARKER,
        FIRST_ACK_MARKER,
    );
    let first_handler =
        FlatGuestImage::new(FIRST_HANDLER, FIRST_HANDLER, &first_handler_bytes)?;
    let second_handler_bytes = build_handler(
        MULTI_DEVICE_SECOND_VIRTUAL_PAGE,
        SECOND_HANDLER_MARKER,
        SECOND_ACK_MARKER,
    );
    let second_handler =
        FlatGuestImage::new(SECOND_HANDLER, SECOND_HANDLER, &second_handler_bytes)?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let mmio_layout = LongModeMmioBootLayout::with_device_mappings(
        memory.region(),
        guest.entry(),
        LONG_MODE_MMIO_STACK_POINTER,
        vec![
            LongModeMmioPageMapping::new(
                LONG_MODE_MMIO_VIRTUAL_PAGE,
                crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR,
            ),
            LongModeMmioPageMapping::new(
                MULTI_DEVICE_SECOND_VIRTUAL_PAGE,
                crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR,
            ),
        ],
    )
    .expect("fixed coupled replay MMIO mappings remain valid");
    let interrupt_layout = LongModeInterruptLayout::with_gates(
        memory.region(),
        guest.entry(),
        LONG_MODE_MMIO_STACK_POINTER,
        vec![
            LongModeInterruptGate::new(TWO_HOST_REGISTRATION_FIRST_VECTOR, first_handler.entry()),
            LongModeInterruptGate::new(TWO_HOST_REGISTRATION_SECOND_VECTOR, second_handler.entry()),
        ],
    )
    .expect("fixed coupled replay interrupt gates remain valid");
    let corrupt_layout = LongModeBootLayout::new(memory.region(), CORRUPT_ENTRY, CORRUPT_STACK)
        .expect("fixed coupled replay corruption layout remains valid");

    interrupt_layout.install_tables(&mut memory)?;
    mmio_layout.install_page_tables(&mut memory)?;
    guest.load(&mut memory)?;
    first_handler.load(&mut memory)?;
    second_handler.load(&mut memory)?;
    initialize_queue_memory(&mut memory, FIRST_QUEUE)?;
    initialize_queue_memory(&mut memory, SECOND_QUEUE)?;
    vm.register_guest_memory(memory)?;

    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(&interrupt_layout)?;
    let _ = vcpu.configure_legacy_pic_extint()?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty coupled replay MSR policy is valid by construction");

    let mut mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR)
        .expect("fixed first coupled replay BAR remains available");
    mmio.register_virtio_blk_device_at(crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR)
        .expect("fixed second coupled replay BAR remains available");
    let first_ready = ready_device(
        crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR,
        FIRST_QUEUE,
    )?;
    let second_ready = ready_device(
        crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR,
        SECOND_QUEUE,
    )?;
    mmio.restore_two_virtio_blk_checkpoints_atomic([
        (
            crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR,
            &first_ready,
        ),
        (
            crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR,
            &second_ready,
        ),
    ])?;

    run_to_capture(&mut vcpu, program.capture_rip)?;
    require_ready_devices(&mmio)?;

    let captured = BoundedFullControllerTwoVirtioBlkCheckpoint::capture(
        &vcpu,
        &vm,
        &msr_policy,
        &mmio,
        [
            crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR,
            crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR,
        ],
        &[
            TRANSACTION_COUPLED_SECOND_PAGE,
            TRANSACTION_COUPLED_FIRST_PAGE,
        ],
    )?;
    require_quiescent_zero_zero(&captured)?;

    let (encoded, checkpoint_encoded_len, registration_pair_encoded_len) = {
        let pair = default_two_host_registration_pair()?;
        let transaction =
            VersionedTwoDeviceCheckpointTransactionV1::from_checkpoint_and_pair(&captured, pair)
                .map_err(|error| coupled_error(error.to_string()))?;
        let checkpoint_encoded_len = transaction
            .checkpoint_encoded_len()
            .map_err(|error| coupled_error(error.to_string()))?;
        let registration_pair_encoded_len = transaction.registration_pair_encoded_len();
        let encoded = transaction
            .encode()
            .map_err(|error| coupled_error(error.to_string()))?;
        (
            encoded,
            checkpoint_encoded_len,
            registration_pair_encoded_len,
        )
    };
    drop(captured);

    let decoded = VersionedTwoDeviceCheckpointTransactionV1::decode(&encoded)
        .map_err(|error| coupled_error(error.to_string()))?;
    let canonical = decoded
        .encode()
        .map_err(|error| coupled_error(error.to_string()))?;
    if canonical != encoded {
        return Err(coupled_error(
            "decoded two-device transaction did not reproduce canonical bytes",
        ));
    }
    let transaction_version = decoded.version();
    let bars = decoded.checkpoint_bars();
    let (checkpoint, registration_pair) = decoded
        .materialize(backend.host_msr_indices())
        .map_err(|error| coupled_error(error.to_string()))?;
    drop(decoded);

    corrupt_owned_state(&checkpoint, &vcpu, &mut vm, &mut mmio, &corrupt_layout)?;
    let mutation = checkpoint.verify(&vcpu, &vm, &mmio)?;
    require_mutation_mismatch(&mutation)?;

    let restored = checkpoint.restore_and_verify(&vcpu, &mut vm, &mut mmio)?;
    require_exact_restore(&restored)?;
    require_restored_zero_zero(&mmio)?;

    let registrations =
        HostRegistrationPairCheckpoint::capture(registration_pair).reconstruct(&backend, &vm)?;
    let replay = run_accelerated_replay(
        &mut vcpu,
        &mut vm,
        &mut mmio,
        &registrations,
        program.completion_rip,
    );
    let cleanup = registrations.deassign(&vm);
    let replay = match (replay, cleanup) {
        (Ok(replay), Ok(())) => replay,
        (_, Err(error)) => return Err(error),
        (Err(error), Ok(())) => return Err(error),
    };

    let replay_queue_indices = [
        queue_indices(&mmio, crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR)?,
        queue_indices(&mmio, crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR)?,
    ];
    if replay_queue_indices != [[1, 1], [1, 1]] {
        return Err(coupled_error(format!(
            "restored queues did not advance independently to 1/1: {replay_queue_indices:?}"
        )));
    }

    let first_readback = read_data(&vm, FIRST_DATA)?;
    let second_readback = read_data(&vm, SECOND_DATA)?;
    let first_backing = mmio
        .virtio_blk_sector_at(crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR)
        .ok_or_else(|| coupled_error("first restored virtio-blk backing disappeared"))?
        .to_vec();
    let second_backing = mmio
        .virtio_blk_sector_at(crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR)
        .ok_or_else(|| coupled_error("second restored virtio-blk backing disappeared"))?
        .to_vec();
    let expected = deterministic_sector();
    if first_readback.as_slice() != expected
        || second_readback.as_slice() != expected
        || first_backing.as_slice() != expected
        || second_backing.as_slice() != expected
    {
        return Err(coupled_error(
            "dual-device replay did not preserve deterministic backing/readback continuity",
        ));
    }

    Ok(TransactionCoupledDualDeviceReplayResult {
        mutation,
        restored,
        transaction_version,
        encoded_len: encoded.len(),
        checkpoint_encoded_len,
        registration_pair_encoded_len,
        bars,
        replay_queue_indices,
        doorbell_events: replay.doorbell_events,
        irqfd_signals: replay.irqfd_signals,
        readback: [first_readback, second_readback],
        backing: [first_backing, second_backing],
        proof: replay.proof,
        completion_rflags: replay.completion_rflags,
    })
}

#[derive(Debug)]
struct GuestProgram {
    bytes: Vec<u8>,
    capture_rip: u64,
    completion_rip: u64,
}

#[derive(Debug)]
struct ReplayEvidence {
    doorbell_events: [u64; 2],
    irqfd_signals: [u32; 2],
    proof: Vec<u8>,
    completion_rflags: u64,
}

fn run_to_capture(vcpu: &mut Vcpu, capture_rip: u64) -> Result<(), Error> {
    let mut port_io = PortIoBus::with_debug_port();
    let io = run_expected_debug_output(vcpu, &mut port_io, CAPTURE_MARKER, "coupled capture")?;
    if io.output_data() != TRANSACTION_COUPLED_CAPTURE_PROOF {
        return Err(coupled_error(
            "unexpected transaction-coupled capture proof",
        ));
    }
    let _ = single_step_to_quiescence(vcpu, capture_rip, "coupled capture quiescence")?;
    Ok(())
}

fn run_accelerated_replay(
    vcpu: &mut Vcpu,
    vm: &mut crate::kvm::Vm,
    mmio: &mut MmioBus,
    registrations: &ReconstructedHostRegistrationPair,
    completion_rip: u64,
) -> Result<ReplayEvidence, Error> {
    let mut port_io = PortIoBus::with_debug_port();
    let mut doorbell_events = [0_u64; 2];
    let mut irqfd_signals = [0_u32; 2];
    let mut completions: [Option<VirtioBlkQueueCompletion>; 2] = [None, None];

    for _ in 0..REPLAY_EXIT_BUDGET {
        let exit = vcpu.run_once()?;
        let disposition = dispatch_vcpu_exit(vcpu, exit, &mut port_io, mmio)?;
        match disposition {
            VmExitDisposition::Continue(continuation) => {
                if is_debug_output(&continuation, FIRST_NOTIFY_MARKER) {
                    service_accelerated_notification(
                        0,
                        crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR,
                        registrations,
                        vm,
                        mmio,
                        &mut doorbell_events,
                        &mut irqfd_signals,
                        &mut completions,
                    )?;
                } else if is_debug_output(&continuation, SECOND_NOTIFY_MARKER) {
                    service_accelerated_notification(
                        1,
                        crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR,
                        registrations,
                        vm,
                        mmio,
                        &mut doorbell_events,
                        &mut irqfd_signals,
                        &mut completions,
                    )?;
                } else if is_debug_output(&continuation, DONE_MARKER) {
                    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
                    if proof.as_slice() != TRANSACTION_COUPLED_REPLAY_PROOF {
                        return Err(coupled_error(format!(
                            "expected replay proof {:?}, got {proof:?}",
                            TRANSACTION_COUPLED_REPLAY_PROOF
                        )));
                    }
                    validate_completion(completions[0], "first")?;
                    validate_completion(completions[1], "second")?;
                    if doorbell_events != [1, 1] || irqfd_signals != [1, 1] {
                        return Err(coupled_error(format!(
                            "expected one ioeventfd and irqfd signal per device, got doorbells={doorbell_events:?} irqfd={irqfd_signals:?}"
                        )));
                    }
                    if mmio.take_device_event_record().is_some() {
                        return Err(coupled_error(
                            "accelerated replay left a userspace MMIO device event",
                        ));
                    }
                    let (_, completion_rflags) = single_step_to_quiescence(
                        vcpu,
                        completion_rip,
                        "coupled replay completion",
                    )?;
                    return Ok(ReplayEvidence {
                        doorbell_events,
                        irqfd_signals,
                        proof,
                        completion_rflags,
                    });
                }
            }
            VmExitDisposition::Stopped(report) => {
                return Err(coupled_error(format!(
                    "dual-device accelerated replay stopped before completion: {report}"
                )));
            }
        }
    }

    Err(coupled_error(
        "dual-device accelerated replay exceeded bounded exit budget",
    ))
}

#[allow(clippy::too_many_arguments)]
fn service_accelerated_notification(
    index: usize,
    bar: u64,
    registrations: &ReconstructedHostRegistrationPair,
    vm: &mut crate::kvm::Vm,
    mmio: &mut MmioBus,
    doorbell_events: &mut [u64; 2],
    irqfd_signals: &mut [u32; 2],
    completions: &mut [Option<VirtioBlkQueueCompletion>; 2],
) -> Result<(), Error> {
    if completions[index].is_some() {
        return Err(coupled_error(format!(
            "device {index} observed duplicate accelerated notification"
        )));
    }
    if mmio.take_device_event_record().is_some() {
        return Err(coupled_error(format!(
            "device {index} ioeventfd notification unexpectedly reached userspace MMIO"
        )));
    }
    let count = registrations.wait_doorbell(index, WAIT_MILLIS)?;
    if count != 1 {
        return Err(coupled_error(format!(
            "device {index} ioeventfd counter was {count}, expected 1"
        )));
    }
    doorbell_events[index] = count;
    if !mmio.apply_virtio_blk_host_notification(bar, 0)? {
        return Err(coupled_error(format!(
            "device {index} lost its restored virtio-blk BAR"
        )));
    }
    let memory = vm
        .guest_memory_mut()
        .ok_or_else(|| coupled_error("restored VM lost registered guest memory"))?;
    let completion = mmio
        .process_virtio_blk_notification(bar, memory)
        .map_err(|error| coupled_error(format!("device {index} queue processing failed: {error}")))?
        .ok_or_else(|| coupled_error(format!("device {index} BAR disappeared during replay")))?;
    completions[index] = Some(completion);
    registrations.signal_irq(index)?;
    irqfd_signals[index] += 1;
    Ok(())
}

fn validate_completion(
    completion: Option<VirtioBlkQueueCompletion>,
    role: &'static str,
) -> Result<(), Error> {
    let completion =
        completion.ok_or_else(|| coupled_error(format!("{role} request never completed")))?;
    if completion.descriptor_id() != 0
        || completion.length() != (VIRTIO_BLK_SECTOR_SIZE + 1) as u32
        || completion.sector() != 0
    {
        return Err(coupled_error(format!(
            "{role} completion mismatch: {completion:?}"
        )));
    }
    Ok(())
}

fn corrupt_owned_state(
    checkpoint: &BoundedFullControllerTwoVirtioBlkCheckpoint,
    vcpu: &Vcpu,
    vm: &mut crate::kvm::Vm,
    mmio: &mut MmioBus,
    corrupt_layout: &LongModeBootLayout,
) -> Result<(), Error> {
    let page = vec![0xa5; LONG_MODE_PAGE_SIZE as usize];
    let memory = vm
        .guest_memory_mut()
        .ok_or_else(|| coupled_error("VM lost guest memory before corruption"))?;
    memory.write(TRANSACTION_COUPLED_FIRST_PAGE, &page)?;
    memory.write(TRANSACTION_COUPLED_SECOND_PAGE, &page)?;

    vcpu.initialize_long_mode(corrupt_layout)?;
    vm.restore_master_pic_state(
        &checkpoint
            .controller()
            .master_pic()
            .with_imr(checkpoint.controller().master_pic().imr() ^ 0x01),
    )?;
    vm.restore_slave_pic_state(
        &checkpoint
            .controller()
            .slave_pic()
            .with_imr(checkpoint.controller().slave_pic().imr() ^ 0x01),
    )?;

    let first = VirtioBlkDevice::new(crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR);
    let second = VirtioBlkDevice::new(crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR);
    mmio.restore_two_virtio_blk_checkpoints_atomic([
        (crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR, &first),
        (crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR, &second),
    ])?;
    Ok(())
}

fn require_mutation_mismatch(
    comparison: &BoundedFullControllerTwoVirtioBlkCheckpointComparison,
) -> Result<(), Error> {
    let controller = comparison.controller();
    if comparison.is_exact_match()
        || controller.page_exact(TRANSACTION_COUPLED_FIRST_PAGE) != Some(false)
        || controller.page_exact(TRANSACTION_COUPLED_SECOND_PAGE) != Some(false)
        || controller.vcpu_exact()
        || controller.master_pic_exact()
        || controller.slave_pic_exact()
        || comparison.device_exact(crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR) != Some(false)
        || comparison.device_exact(crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR) != Some(false)
    {
        return Err(coupled_error(format!(
            "expected page/vcpu/PIC/device mutation mismatch, got first-page={:?} second-page={:?} vcpu={} master={} slave={} first-device={:?} second-device={:?}",
            controller.page_exact(TRANSACTION_COUPLED_FIRST_PAGE),
            controller.page_exact(TRANSACTION_COUPLED_SECOND_PAGE),
            controller.vcpu_exact(),
            controller.master_pic_exact(),
            controller.slave_pic_exact(),
            comparison.device_exact(crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR),
            comparison.device_exact(crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR),
        )));
    }
    Ok(())
}

fn require_exact_restore(
    comparison: &BoundedFullControllerTwoVirtioBlkCheckpointComparison,
) -> Result<(), Error> {
    if !comparison.is_exact_match()
        || comparison
            .controller()
            .page_exact(TRANSACTION_COUPLED_FIRST_PAGE)
            != Some(true)
        || comparison
            .controller()
            .page_exact(TRANSACTION_COUPLED_SECOND_PAGE)
            != Some(true)
        || comparison.device_exact(crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR) != Some(true)
        || comparison.device_exact(crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR) != Some(true)
    {
        return Err(coupled_error("transaction restore was not exact"));
    }
    Ok(())
}

fn require_ready_devices(mmio: &MmioBus) -> Result<(), Error> {
    let expected_status = VIRTIO_STATUS_ACKNOWLEDGE
        | VIRTIO_STATUS_DRIVER
        | VIRTIO_STATUS_FEATURES_OK
        | VIRTIO_STATUS_DRIVER_OK;
    for bar in [
        crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR,
        crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR,
    ] {
        if mmio.virtio_blk_status_at(bar) != Some(expected_status)
            || mmio.virtio_blk_driver_features_at(bar) != Some(VIRTIO_F_VERSION_1)
            || mmio.virtio_blk_queue_enabled_at(bar) != Some(true)
        {
            return Err(coupled_error(format!(
                "virtio-blk BAR {bar:#x} was not queue-ready before capture"
            )));
        }
    }
    Ok(())
}

fn require_quiescent_zero_zero(
    checkpoint: &BoundedFullControllerTwoVirtioBlkCheckpoint,
) -> Result<(), Error> {
    for bar in checkpoint.device_bars() {
        let device = checkpoint
            .device(bar)
            .ok_or_else(|| coupled_error(format!("captured BAR {bar:#x} disappeared")))?;
        if device.checkpoint_last_avail_idx() != 0
            || device.checkpoint_last_used_idx() != 0
            || !device.checkpoint_quiescent()
        {
            return Err(coupled_error(format!(
                "captured BAR {bar:#x} is not quiescent 0/0"
            )));
        }
    }
    Ok(())
}

fn require_restored_zero_zero(mmio: &MmioBus) -> Result<(), Error> {
    for bar in [
        crate::kvm::sys::TWO_HOST_REGISTRATION_FIRST_BAR,
        crate::kvm::sys::TWO_HOST_REGISTRATION_SECOND_BAR,
    ] {
        if queue_indices(mmio, bar)? != [0, 0] {
            return Err(coupled_error(format!(
                "restored BAR {bar:#x} did not return to queue 0/0"
            )));
        }
    }
    Ok(())
}

fn queue_indices(mmio: &MmioBus, bar: u64) -> Result<[u16; 2], Error> {
    let device = mmio
        .capture_virtio_blk_checkpoint_at(bar)?
        .ok_or_else(|| coupled_error(format!("virtio-blk BAR {bar:#x} disappeared")))?;
    if !device.checkpoint_quiescent() {
        return Err(coupled_error(format!(
            "virtio-blk BAR {bar:#x} is not quiescent"
        )));
    }
    Ok([
        device.checkpoint_last_avail_idx(),
        device.checkpoint_last_used_idx(),
    ])
}

fn ready_device(bar: u64, queue: QueueLayout) -> Result<VirtioBlkDevice, Error> {
    let mut device = VirtioBlkDevice::new(bar);
    device_write(&mut device, 0x14, &[VIRTIO_STATUS_ACKNOWLEDGE])?;
    device_write(
        &mut device,
        0x14,
        &[VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER],
    )?;
    device_write(&mut device, 0x08, &1_u32.to_le_bytes())?;
    device_write(&mut device, 0x0c, &1_u32.to_le_bytes())?;
    device_write(
        &mut device,
        0x14,
        &[VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK],
    )?;
    device_write(&mut device, 0x16, &0_u16.to_le_bytes())?;
    device_write(&mut device, 0x18, &QUEUE_SIZE.to_le_bytes())?;
    device_write(&mut device, 0x20, &(queue.desc as u32).to_le_bytes())?;
    device_write(
        &mut device,
        0x24,
        &((queue.desc >> 32) as u32).to_le_bytes(),
    )?;
    device_write(&mut device, 0x28, &(queue.avail as u32).to_le_bytes())?;
    device_write(
        &mut device,
        0x2c,
        &((queue.avail >> 32) as u32).to_le_bytes(),
    )?;
    device_write(&mut device, 0x30, &(queue.used as u32).to_le_bytes())?;
    device_write(
        &mut device,
        0x34,
        &((queue.used >> 32) as u32).to_le_bytes(),
    )?;
    device_write(&mut device, 0x1c, &1_u16.to_le_bytes())?;
    device_write(
        &mut device,
        0x14,
        &[VIRTIO_STATUS_ACKNOWLEDGE
            | VIRTIO_STATUS_DRIVER
            | VIRTIO_STATUS_FEATURES_OK
            | VIRTIO_STATUS_DRIVER_OK],
    )?;
    if !device.queue_enabled()
        || !device.checkpoint_quiescent()
        || device.checkpoint_last_avail_idx() != 0
        || device.checkpoint_last_used_idx() != 0
    {
        return Err(coupled_error(format!(
            "prepared BAR {bar:#x} did not reach queue-ready 0/0 state"
        )));
    }
    Ok(device)
}

fn device_write(device: &mut VirtioBlkDevice, offset: u64, payload: &[u8]) -> Result<(), Error> {
    if device
        .write(offset, payload)
        .map_err(|error| coupled_error(error.to_string()))?
        .is_some()
    {
        return Err(coupled_error(format!(
            "configuration write at offset {offset:#x} unexpectedly emitted a device event"
        )));
    }
    Ok(())
}

fn initialize_queue_memory(memory: &mut GuestMemory, queue: QueueLayout) -> Result<(), Error> {
    write_descriptor(memory, queue, 0, queue.header, 16, VIRTQ_DESC_F_NEXT, 1)?;
    write_descriptor(
        memory,
        queue,
        1,
        queue.data,
        VIRTIO_BLK_SECTOR_SIZE as u32,
        VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE,
        2,
    )?;
    write_descriptor(memory, queue, 2, queue.status, 1, VIRTQ_DESC_F_WRITE, 0)?;

    let header = [0_u8; 16];
    memory.write(GuestPhysAddr::new(queue.header), &header)?;
    memory.write(
        GuestPhysAddr::new(queue.data),
        &vec![0_u8; VIRTIO_BLK_SECTOR_SIZE],
    )?;
    memory.write(GuestPhysAddr::new(queue.status), &[0xff])?;
    memory.write(GuestPhysAddr::new(queue.avail), &[0, 0, 0, 0, 0, 0, 0, 0])?;
    memory.write(
        GuestPhysAddr::new(queue.used),
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    )?;
    Ok(())
}

fn write_descriptor(
    memory: &mut GuestMemory,
    queue: QueueLayout,
    index: u16,
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
) -> Result<(), Error> {
    let mut descriptor = [0_u8; 16];
    descriptor[0..8].copy_from_slice(&address.to_le_bytes());
    descriptor[8..12].copy_from_slice(&length.to_le_bytes());
    descriptor[12..14].copy_from_slice(&flags.to_le_bytes());
    descriptor[14..16].copy_from_slice(&next.to_le_bytes());
    memory.write(
        GuestPhysAddr::new(queue.desc + 16 * u64::from(index)),
        &descriptor,
    )
}

fn read_data(vm: &crate::kvm::Vm, address: u64) -> Result<Vec<u8>, Error> {
    let memory = vm
        .guest_memory()
        .ok_or_else(|| coupled_error("restored VM lost guest memory during readback"))?;
    let mut data = vec![0_u8; VIRTIO_BLK_SECTOR_SIZE];
    memory.read(GuestPhysAddr::new(address), &mut data)?;
    Ok(data)
}

fn build_guest_program() -> GuestProgram {
    let mut code = Vec::new();
    emit_pic_setup(&mut code);
    code.extend_from_slice(&[0xfb, 0x90]);
    emit_debug(&mut code, CAPTURE_MARKER);
    let capture_rip = ENTRY.get() + code.len() as u64;
    code.push(0x90);

    emit_request(
        &mut code,
        FIRST_AVAIL,
        LONG_MODE_MMIO_VIRTUAL_PAGE,
        FIRST_QUEUE,
        FIRST_NOTIFY_MARKER,
        FIRST_RESUMED_MARKER,
    );
    emit_request(
        &mut code,
        SECOND_AVAIL,
        MULTI_DEVICE_SECOND_VIRTUAL_PAGE,
        SECOND_QUEUE,
        SECOND_NOTIFY_MARKER,
        SECOND_RESUMED_MARKER,
    );
    emit_debug(&mut code, DONE_MARKER);
    let completion_rip = ENTRY.get() + code.len() as u64;
    code.push(0x90);
    code.push(0xf4);

    GuestProgram {
        bytes: code,
        capture_rip,
        completion_rip,
    }
}

fn emit_request(
    code: &mut Vec<u8>,
    avail: u64,
    virtual_bar: u64,
    queue: QueueLayout,
    notify_marker: u8,
    resumed_marker: u8,
) {
    code.push(0xfa);
    emit_movabs(code, 7, avail);
    code.extend_from_slice(&[0x66, 0xc7, 0x47, 0x02, 0x01, 0x00]);
    emit_movabs(code, 3, virtual_bar);
    code.extend_from_slice(&[0x66, 0xc7, 0x83, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);
    emit_debug(code, notify_marker);
    code.extend_from_slice(&[0xfb, 0xf4]);
    emit_guest_completion_checks(code, queue);
    emit_debug(code, resumed_marker);
}

fn build_handler(virtual_bar: u64, handler_marker: u8, ack_marker: u8) -> Vec<u8> {
    let mut code = Vec::new();
    emit_debug(&mut code, handler_marker);
    emit_movabs(&mut code, 3, virtual_bar);
    code.extend_from_slice(&[0x8a, 0x83]);
    code.extend_from_slice(&(VIRTIO_ISR_OFFSET as u32).to_le_bytes());
    emit_cmp_al(&mut code, VIRTIO_ISR_QUEUE_INTERRUPT);
    emit_debug(&mut code, ack_marker);
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
    code.extend_from_slice(&[0xb0, 0xfc, 0xe6, 0x21]);
    code.extend_from_slice(&[0xb0, 0xff, 0xe6, 0xa1]);
}

fn emit_guest_completion_checks(code: &mut Vec<u8>, queue: QueueLayout) {
    emit_movabs(code, 7, queue.used);
    code.extend_from_slice(&[0x0f, 0xb7, 0x47, 0x02, 0x83, 0xf8, 0x01]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x04, 0x85, 0xc0]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x08]);
    emit_cmp_eax(code, (VIRTIO_BLK_SECTOR_SIZE + 1) as u32);
    emit_movabs(code, 7, queue.status);
    code.extend_from_slice(&[0x8a, 0x07]);
    emit_cmp_al(code, VIRTIO_BLK_S_OK);
    emit_movabs(code, 7, queue.data);
    code.extend_from_slice(&[0x48, 0x8b, 0x07]);
    emit_movabs(code, 1, u64::from_le_bytes(*b"BLK-SECT"));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x48, 0x8b, 0x87, 0xf8, 0x01, 0x00, 0x00]);
    emit_movabs(code, 1, u64::from_le_bytes(*b"BLKEND!!"));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    emit_equal_or_ud2(code);
}

fn emit_debug(code: &mut Vec<u8>, byte: u8) {
    code.extend_from_slice(&[0xb0, byte, 0xe6, 0xe9]);
}

fn emit_movabs(code: &mut Vec<u8>, register: u8, value: u64) {
    debug_assert!(register < 8);
    code.extend_from_slice(&[0x48, 0xb8 + register]);
    code.extend_from_slice(&value.to_le_bytes());
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

fn run_expected_debug_output(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    expected: u8,
    stage: &'static str,
) -> Result<crate::vcpu::PortIoExit, Error> {
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Io {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage,
            expected_reason: VcpuExit::Io.reason(),
            actual_reason: exit.reason(),
        }));
    }
    let io = vcpu.port_io_exit()?;
    if io.direction() != PortIoDirection::Out
        || io.port() != DEBUG_PORT
        || io.size() != 1
        || io.count() != 1
        || io.output_data() != [expected]
    {
        return Err(coupled_error(format!(
            "{stage}: expected debug byte {:?}, got {io:?}",
            char::from(expected)
        )));
    }
    let _ = port_io.dispatch(&io)?;
    Ok(io)
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

fn single_step_to_quiescence(
    vcpu: &mut Vcpu,
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
        return Err(coupled_error(format!(
            "{stage}: expected debug exit, got reason {}",
            exit.reason()
        )));
    }
    let registers = vcpu.registers()?;
    if registers.rip != expected_rip
        || registers.rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || registers.rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
    {
        return Err(coupled_error(format!(
            "{stage}: expected IF-set quiescent rip={expected_rip:#x}, got rip={:#x} rflags={:#x}",
            registers.rip, registers.rflags
        )));
    }
    Ok((registers.rip, registers.rflags))
}

fn coupled_error(detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation: "transaction-coupled dual-device replay",
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_pages_are_distinct_aligned_and_owned_by_the_transaction() {
        assert_eq!(TRANSACTION_COUPLED_FIRST_PAGE.get(), FIRST_DESC);
        assert_eq!(TRANSACTION_COUPLED_SECOND_PAGE.get(), SECOND_DESC);
        assert_eq!(FIRST_DESC % LONG_MODE_PAGE_SIZE, 0);
        assert_eq!(SECOND_DESC % LONG_MODE_PAGE_SIZE, 0);
        assert_eq!(SECOND_DESC - FIRST_DESC, LONG_MODE_PAGE_SIZE);
    }

    #[test]
    fn guest_contract_has_two_accelerated_notifications_and_stable_proof() {
        let program = build_guest_program();
        assert!(program.capture_rip < program.completion_rip);
        assert_eq!(TRANSACTION_COUPLED_CAPTURE_PROOF, b"C");
        assert_eq!(TRANSACTION_COUPLED_REPLAY_PROOF, b"A0aMB1bND");
        assert!(program
            .bytes
            .windows(9)
            .any(|window| { window == [0x66, 0xc7, 0x83, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00] }));
    }
}
