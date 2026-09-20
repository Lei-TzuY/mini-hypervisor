use super::super::transaction_coupled_dual_device_replay::{
    emit_cmp_al, emit_debug, emit_equal_or_ud2, emit_movabs, is_debug_output, queue_indices,
    read_data, ready_device, require_ready_devices, require_restored_zero_zero,
    single_step_to_quiescence, QueueLayout, FIRST_QUEUE, SECOND_QUEUE,
    TRANSACTION_COUPLED_FIRST_PAGE, TRANSACTION_COUPLED_SECOND_PAGE,
};
use super::super::transaction_coupled_dual_device_replay::write_readback::{
    emit_write_then_read, initialize_write_queue_memory, second_write_readback_sector,
    service_write_readback_notification, RequestKind,
};
use crate::interrupt::{LongModeInterruptGate, LongModeInterruptLayout};
use crate::mmio::long_mode::{
    LongModeMmioBootLayout, LongModeMmioPageMapping, LONG_MODE_MMIO_VIRTUAL_PAGE,
};
use crate::mmio::multi_device::MULTI_DEVICE_SECOND_VIRTUAL_PAGE;
use crate::portio::pci::virtio::{VIRTIO_ISR_OFFSET, VIRTIO_ISR_QUEUE_INTERRUPT};
use crate::portio::virtio_blk_fixture::deterministic_write_readback_sector;
use crate::vmexit::{dispatch_vcpu_exit, VmExitDisposition};

pub const RESTORED_MULTI_PRODUCER_FIRST_GSI: u32 = 16;
pub const RESTORED_MULTI_PRODUCER_SECOND_GSI: u32 = 17;
pub const RESTORED_MULTI_PRODUCER_FIRST_VECTOR: u8 = 0x50;
pub const RESTORED_MULTI_PRODUCER_SECOND_VECTOR: u8 = 0x51;
pub const RESTORED_MULTI_PRODUCER_FIRST_PROOF: &[u8; 9] = b"W0aMR0aXD";
pub const RESTORED_MULTI_PRODUCER_SECOND_PROOF: &[u8; 9] = b"Y1bNZ1bQE";

const FIRST_HANDLER_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x14000);
const SECOND_HANDLER_ENTRY: GuestPhysAddr = GuestPhysAddr::new(0x15000);
const LAPIC_VIRTUAL_PAGE: u64 = 0x502000;
const LAPIC_GPA: u64 = 0xfee0_0000;
const LAPIC_EOI_OFFSET: u32 = 0x0b0;
const PRODUCER_EXIT_BUDGET: u32 = 32;
const FIRST_WRITE_NOTIFY: u8 = b'W';
const FIRST_READ_NOTIFY: u8 = b'R';
const FIRST_WRITE_RESUMED: u8 = b'M';
const FIRST_READ_RESUMED: u8 = b'X';
const SECOND_WRITE_NOTIFY: u8 = b'Y';
const SECOND_READ_NOTIFY: u8 = b'Z';
const SECOND_WRITE_RESUMED: u8 = b'N';
const SECOND_READ_RESUMED: u8 = b'Q';
const FIRST_HANDLER_MARKER: u8 = b'0';
const FIRST_ACK_MARKER: u8 = b'a';
const SECOND_HANDLER_MARKER: u8 = b'1';
const SECOND_ACK_MARKER: u8 = b'b';
const FIRST_DONE_MARKER: u8 = b'D';
const SECOND_DONE_MARKER: u8 = b'E';
const IOAPIC_DESTINATION_SHIFT: u32 = 56;

const MULTI_PRODUCER_OWNERSHIP_SET: [GuestPhysAddr; 5] = [
    TRANSACTION_COUPLED_FIRST_PAGE,
    TRANSACTION_COUPLED_SECOND_PAGE,
    TWO_VCPU_CHECKPOINT_SHARED_PAGE,
    TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE,
    TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredMultiProducerDataPlaneReplayResult {
    mutation: BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison,
    restored: BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison,
    schema_version: u16,
    encoded_len: usize,
    page_count: usize,
    canonical_roundtrip: bool,
    bars: [u64; 2],
    ioapic_entries: [u64; 2],
    capture_pending: [bool; 2],
    reconstructed_pending: [bool; 2],
    queue_indices: [[u16; 2]; 2],
    doorbell_events: [u64; 2],
    irqfd_signals: [u32; 2],
    write_payloads: [Vec<u8>; 2],
    readback: [Vec<u8>; 2],
    backing: [Vec<u8>; 2],
    producer_proofs: [Vec<u8>; 2],
    capture_rips: [u64; 2],
    completion_rips: [u64; 2],
}

impl RestoredMultiProducerDataPlaneReplayResult {
    #[must_use]
    pub const fn mutation(&self) -> &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison {
        &self.mutation
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison {
        &self.restored
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn encoded_len(&self) -> usize {
        self.encoded_len
    }

    #[must_use]
    pub const fn page_count(&self) -> usize {
        self.page_count
    }

    #[must_use]
    pub const fn canonical_roundtrip(&self) -> bool {
        self.canonical_roundtrip
    }

    #[must_use]
    pub const fn bars(&self) -> [u64; 2] {
        self.bars
    }

    #[must_use]
    pub const fn ioapic_entries(&self) -> [u64; 2] {
        self.ioapic_entries
    }

    #[must_use]
    pub const fn capture_pending(&self) -> [bool; 2] {
        self.capture_pending
    }

    #[must_use]
    pub const fn reconstructed_pending(&self) -> [bool; 2] {
        self.reconstructed_pending
    }

    #[must_use]
    pub const fn queue_indices(&self) -> [[u16; 2]; 2] {
        self.queue_indices
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
    pub fn write_payloads(&self) -> [&[u8]; 2] {
        [&self.write_payloads[0], &self.write_payloads[1]]
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
    pub fn producer_proofs(&self) -> [&[u8]; 2] {
        [&self.producer_proofs[0], &self.producer_proofs[1]]
    }

    #[must_use]
    pub const fn capture_rips(&self) -> [u64; 2] {
        self.capture_rips
    }

    #[must_use]
    pub const fn completion_rips(&self) -> [u64; 2] {
        self.completion_rips
    }
}

pub fn run_restored_multi_producer_data_plane_replay_guest(
) -> Result<RestoredMultiProducerDataPlaneReplayResult, Error> {
    let payloads = [
        deterministic_write_readback_sector(),
        second_write_readback_sector(),
    ];
    if payloads[0] == payloads[1] {
        return Err(page_set_error(
            "restored multi-producer payload isolation",
            "producer payloads unexpectedly alias",
        ));
    }

    let first_program = build_bound_producer_program(0, FIRST_QUEUE, &payloads[0]);
    let second_program = build_bound_producer_program(1, SECOND_QUEUE, &payloads[1]);
    let first_image = FlatGuestImage::new(
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        &first_program.bytes,
    )?;
    let second_image = FlatGuestImage::new(
        TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
        TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
        &second_program.bytes,
    )?;
    let first_handler_bytes = build_bound_handler(
        LONG_MODE_MMIO_VIRTUAL_PAGE,
        FIRST_HANDLER_MARKER,
        FIRST_ACK_MARKER,
    );
    let second_handler_bytes = build_bound_handler(
        MULTI_DEVICE_SECOND_VIRTUAL_PAGE,
        SECOND_HANDLER_MARKER,
        SECOND_ACK_MARKER,
    );
    let first_handler =
        FlatGuestImage::new(FIRST_HANDLER_ENTRY, FIRST_HANDLER_ENTRY, &first_handler_bytes)?;
    let second_handler =
        FlatGuestImage::new(SECOND_HANDLER_ENTRY, SECOND_HANDLER_ENTRY, &second_handler_bytes)?;

    let backend = crate::kvm::KvmBackend::open()?;
    backend.require_mp_state_capability()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;

    let first_interrupt_layout = LongModeInterruptLayout::with_gates(
        memory.region(),
        first_image.entry(),
        TWO_VCPU_CHECKPOINT_FIRST_STACK,
        interrupt_gates(first_handler.entry(), second_handler.entry()),
    )
    .expect("fixed first multi-producer interrupt layout remains valid");
    let second_interrupt_layout = LongModeInterruptLayout::with_gates(
        memory.region(),
        second_image.entry(),
        TWO_VCPU_CHECKPOINT_SECOND_STACK,
        interrupt_gates(first_handler.entry(), second_handler.entry()),
    )
    .expect("fixed second multi-producer interrupt layout remains valid");
    let mmio_layout = LongModeMmioBootLayout::with_device_mappings(
        memory.region(),
        first_image.entry(),
        TWO_VCPU_CHECKPOINT_FIRST_STACK,
        vec![
            LongModeMmioPageMapping::new(
                LONG_MODE_MMIO_VIRTUAL_PAGE,
                TWO_HOST_REGISTRATION_FIRST_BAR,
            ),
            LongModeMmioPageMapping::new(
                MULTI_DEVICE_SECOND_VIRTUAL_PAGE,
                TWO_HOST_REGISTRATION_SECOND_BAR,
            ),
            LongModeMmioPageMapping::new(LAPIC_VIRTUAL_PAGE, LAPIC_GPA),
        ],
    )
    .expect("fixed multi-producer MMIO mappings remain valid");
    let corrupt_first_layout = LongModeBootLayout::new(
        memory.region(),
        CORRUPT_FIRST_ENTRY,
        CORRUPT_FIRST_STACK,
    )
    .expect("fixed first multi-producer corruption layout remains valid");
    let corrupt_second_layout = LongModeBootLayout::new(
        memory.region(),
        CORRUPT_SECOND_ENTRY,
        CORRUPT_SECOND_STACK,
    )
    .expect("fixed second multi-producer corruption layout remains valid");

    first_interrupt_layout.install_tables(&mut memory)?;
    mmio_layout.install_page_tables(&mut memory)?;
    first_image.load(&mut memory)?;
    second_image.load(&mut memory)?;
    first_handler.load(&mut memory)?;
    second_handler.load(&mut memory)?;
    initialize_write_queue_memory(&mut memory, FIRST_QUEUE, &payloads[0])?;
    initialize_write_queue_memory(&mut memory, SECOND_QUEUE, &payloads[1])?;
    vm.register_guest_memory(memory)?;

    let mut first = vm.create_vcpu(TWO_VCPU_CHECKPOINT_FIRST_ID)?;
    let mut second = vm.create_vcpu(TWO_VCPU_CHECKPOINT_SECOND_ID)?;
    first.initialize_long_mode_interrupts(&first_interrupt_layout)?;
    second.initialize_long_mode_interrupts(&second_interrupt_layout)?;
    let _ = first.configure_legacy_pic_extint()?;
    let _ = second.configure_legacy_pic_extint()?;
    let first_mp = first.ensure_runnable_mp_state()?;
    let second_mp = second.ensure_runnable_mp_state()?;
    if [first_mp, second_mp] != [MP_STATE_RUNNABLE, MP_STATE_RUNNABLE] {
        return Err(page_set_error(
            "restored multi-producer MP-state preparation",
            format!("expected RUNNABLE [0, 0], got [{first_mp}, {second_mp}]"),
        ));
    }
    configure_multi_producer_ioapic(&vm)?;

    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty restored multi-producer MSR policy is valid");
    let mut mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_FIRST_BAR)
        .expect("fixed first multi-producer BAR remains available");
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_SECOND_BAR)
        .expect("fixed second multi-producer BAR remains available");
    let first_ready = ready_device(TWO_HOST_REGISTRATION_FIRST_BAR, FIRST_QUEUE)?;
    let second_ready = ready_device(TWO_HOST_REGISTRATION_SECOND_BAR, SECOND_QUEUE)?;
    mmio.restore_two_virtio_blk_checkpoints_atomic([
        (TWO_HOST_REGISTRATION_FIRST_BAR, &first_ready),
        (TWO_HOST_REGISTRATION_SECOND_BAR, &second_ready),
    ])?;
    require_ready_devices(&mmio)?;

    let pair = multi_producer_registration_pair()?;
    let capture_registrations = HostRegistrationPairCheckpoint::capture(pair)
        .reconstruct(&backend, &vm)?;
    let (first_capture_rip, _) = two_vcpu_two_device_retired_barrier(
        &mut first,
        FIRST_CAPTURE_MARKER,
        first_program.capture_rip,
        "first restored multi-producer capture barrier",
    )?;
    let (second_capture_rip, _) = two_vcpu_two_device_retired_barrier(
        &mut second,
        SECOND_CAPTURE_MARKER,
        second_program.capture_rip,
        "second restored multi-producer capture barrier",
    )?;
    let capture_pending = capture_registrations.pending_doorbells()?;
    if capture_pending != [false, false] {
        return two_vcpu_two_device_cleanup_error(
            capture_registrations,
            &vm,
            page_set_error(
                "restored multi-producer capture quiescence",
                format!("expected no pending doorbells, got {capture_pending:?}"),
            ),
        );
    }

    let transaction_result = TwoVcpuTwoDeviceCheckpointTransaction::capture(
        TwoVcpuTwoDeviceCaptureContext {
            first: &first,
            second: &second,
            vm: &vm,
            msr_policy: &msr_policy,
            mmio: &mmio,
            bars: [
                TWO_HOST_REGISTRATION_SECOND_BAR,
                TWO_HOST_REGISTRATION_FIRST_BAR,
            ],
            page_addresses: &MULTI_PRODUCER_OWNERSHIP_SET,
        },
        pair,
        &capture_registrations,
    );
    let capture_cleanup = capture_registrations.deassign(&vm);
    let transaction = match (transaction_result, capture_cleanup) {
        (Ok(transaction), Ok(())) => transaction,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup_error)) => {
            return Err(page_set_error(
                "restored multi-producer capture cleanup",
                format!("capture failed: {error}; cleanup also failed: {cleanup_error}"),
            ))
        }
    };

    let (transaction, transport) = prepare_two_vcpu_two_device_transaction_transport(
        transaction,
        pair,
        &backend,
        TwoVcpuTwoDeviceTransactionTransport::VersionedV1,
    )?;
    let transport = transport.expect("versioned multi-producer transport always returns evidence");
    if !transport.canonical_roundtrip || transport.page_count != MULTI_PRODUCER_OWNERSHIP_SET.len() {
        return Err(page_set_error(
            "restored multi-producer versioned ownership",
            format!(
                "expected canonical five-page transport, got canonical={} pages={}",
                transport.canonical_roundtrip, transport.page_count
            ),
        ));
    }

    corrupt_multi_producer_pages(&mut vm)?;
    first.initialize_long_mode(&corrupt_first_layout)?;
    second.initialize_long_mode(&corrupt_second_layout)?;
    first.restore_multiprocessing_state_raw(MP_STATE_HALTED)?;
    second.restore_multiprocessing_state_raw(MP_STATE_UNINITIALIZED)?;
    two_vcpu_two_device_corrupt_controller(&first, &second, &vm)?;
    let first_corrupt = VirtioBlkDevice::new(TWO_HOST_REGISTRATION_FIRST_BAR);
    let second_corrupt = VirtioBlkDevice::new(TWO_HOST_REGISTRATION_SECOND_BAR);
    mmio.restore_two_virtio_blk_checkpoints_atomic([
        (TWO_HOST_REGISTRATION_FIRST_BAR, &first_corrupt),
        (TWO_HOST_REGISTRATION_SECOND_BAR, &second_corrupt),
    ])?;

    let mutation = transaction.checkpoint().verify(&first, &second, &vm, &mmio)?;
    require_multi_producer_mutation(&mutation)?;

    let (restored, registrations) =
        transaction.restore_and_reconstruct(&backend, &mut first, &mut second, &mut vm, &mut mmio)?;
    if !restored.is_exact_match() {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "restored multi-producer exact restore",
                "materialized transaction did not restore exactly",
            ),
        );
    }
    require_ready_devices(&mmio)?;
    require_restored_zero_zero(&mmio)?;
    let reconstructed_pending = registrations.pending_doorbells()?;
    if reconstructed_pending != [false, false] {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "restored multi-producer reconstructed quiescence",
                format!("expected no pending doorbells, got {reconstructed_pending:?}"),
            ),
        );
    }
    let ioapic_entries = multi_producer_ioapic_entries(&vm)?;
    let expected_ioapic = expected_multi_producer_ioapic_entries();
    if ioapic_entries != expected_ioapic {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "restored multi-producer IOAPIC routing",
                format!("expected {expected_ioapic:?}, got {ioapic_entries:?}"),
            ),
        );
    }

    let mut doorbell_events = [0_u64; 2];
    let mut irqfd_signals = [0_u32; 2];
    let mut completions = [None; 4];
    let replay = (|| -> Result<([Vec<u8>; 2], [u64; 2]), Error> {
        let first_proof = run_bound_producer(
            &mut first,
            0,
            &mut vm,
            &mut mmio,
            &registrations,
            &payloads,
            &mut doorbell_events,
            &mut irqfd_signals,
            &mut completions,
            first_program.completion_rip,
        )?;
        let second_proof = run_bound_producer(
            &mut second,
            1,
            &mut vm,
            &mut mmio,
            &registrations,
            &payloads,
            &mut doorbell_events,
            &mut irqfd_signals,
            &mut completions,
            second_program.completion_rip,
        )?;
        Ok((
            [first_proof, second_proof],
            [first_program.completion_rip, second_program.completion_rip],
        ))
    })();
    let cleanup = registrations.deassign(&vm);
    let (producer_proofs, completion_rips) = match (replay, cleanup) {
        (Ok(evidence), Ok(())) => evidence,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup_error)) => {
            return Err(page_set_error(
                "restored multi-producer replay cleanup",
                format!("replay failed: {error}; cleanup also failed: {cleanup_error}"),
            ))
        }
    };

    if doorbell_events != [2, 2] || irqfd_signals != [2, 2] {
        return Err(page_set_error(
            "restored multi-producer accelerated counts",
            format!(
                "expected two doorbells/irqfd signals per producer, got doorbells={doorbell_events:?} irqfd={irqfd_signals:?}"
            ),
        ));
    }
    let queue_indices = [
        queue_indices(&mmio, TWO_HOST_REGISTRATION_FIRST_BAR)?,
        queue_indices(&mmio, TWO_HOST_REGISTRATION_SECOND_BAR)?,
    ];
    if queue_indices != [[2, 2], [2, 2]] {
        return Err(page_set_error(
            "restored multi-producer queue ownership",
            format!("expected both queues at 2/2, got {queue_indices:?}"),
        ));
    }

    let readback = [
        read_data(&vm, FIRST_QUEUE.data)?,
        read_data(&vm, SECOND_QUEUE.data)?,
    ];
    let backing = [
        mmio.virtio_blk_sector_at(TWO_HOST_REGISTRATION_FIRST_BAR)
            .ok_or_else(|| page_set_error("first multi-producer backing", "device disappeared"))?
            .to_vec(),
        mmio.virtio_blk_sector_at(TWO_HOST_REGISTRATION_SECOND_BAR)
            .ok_or_else(|| page_set_error("second multi-producer backing", "device disappeared"))?
            .to_vec(),
    ];
    for index in 0..2 {
        if readback[index].as_slice() != payloads[index]
            || backing[index].as_slice() != payloads[index]
        {
            return Err(page_set_error(
                "restored multi-producer backing isolation",
                format!("producer/device {index} did not preserve its own payload"),
            ));
        }
    }
    if backing[0] == backing[1] {
        return Err(page_set_error(
            "restored multi-producer backing isolation",
            "distinct producer backings unexpectedly alias",
        ));
    }

    Ok(RestoredMultiProducerDataPlaneReplayResult {
        mutation,
        restored,
        schema_version: transport.schema_version,
        encoded_len: transport.encoded_len,
        page_count: transport.page_count,
        canonical_roundtrip: transport.canonical_roundtrip,
        bars: transport.bars,
        ioapic_entries,
        capture_pending,
        reconstructed_pending,
        queue_indices,
        doorbell_events,
        irqfd_signals,
        write_payloads: [payloads[0].to_vec(), payloads[1].to_vec()],
        readback,
        backing,
        producer_proofs,
        capture_rips: [first_capture_rip, second_capture_rip],
        completion_rips,
    })
}

#[derive(Debug)]
struct BoundProducerProgram {
    bytes: Vec<u8>,
    capture_rip: u64,
    completion_rip: u64,
}

fn build_bound_producer_program(
    index: usize,
    queue: QueueLayout,
    payload: &[u8; crate::portio::pci::virtio_blk::VIRTIO_BLK_SECTOR_SIZE],
) -> BoundProducerProgram {
    let (entry, marker, capture, virtual_bar, write_notify, write_resumed, read_notify, read_resumed, done) =
        if index == 0 {
            (
                TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
                TWO_VCPU_CHECKPOINT_FIRST_MARKER,
                FIRST_CAPTURE_MARKER,
                LONG_MODE_MMIO_VIRTUAL_PAGE,
                FIRST_WRITE_NOTIFY,
                FIRST_WRITE_RESUMED,
                FIRST_READ_NOTIFY,
                FIRST_READ_RESUMED,
                FIRST_DONE_MARKER,
            )
        } else {
            (
                TWO_VCPU_CHECKPOINT_SECOND_ENTRY,
                TWO_VCPU_CHECKPOINT_SECOND_MARKER,
                SECOND_CAPTURE_MARKER,
                MULTI_DEVICE_SECOND_VIRTUAL_PAGE,
                SECOND_WRITE_NOTIFY,
                SECOND_WRITE_RESUMED,
                SECOND_READ_NOTIFY,
                SECOND_READ_RESUMED,
                SECOND_DONE_MARKER,
            )
        };
    let mut code = Vec::new();
    if index == 0 {
        code.extend_from_slice(&[
            0xc6,
            0x04,
            0x25,
            0x00,
            0x00,
            0x03,
            0x00,
            TWO_VCPU_CHECKPOINT_SHARED_MARKER,
        ]);
    }
    code.extend_from_slice(&[0x6a, marker]);
    emit_debug(&mut code, capture);
    let capture_rip = entry.get() + code.len() as u64;
    code.push(0x90);

    code.push(0x58);
    emit_cmp_al(&mut code, marker);
    code.extend_from_slice(&[0x8a, 0x04, 0x25, 0x00, 0x00, 0x03, 0x00]);
    emit_cmp_al(&mut code, TWO_VCPU_CHECKPOINT_SHARED_MARKER);
    emit_write_then_read(
        &mut code,
        queue,
        virtual_bar,
        write_notify,
        write_resumed,
        read_notify,
        read_resumed,
        payload,
    );
    emit_debug(&mut code, done);
    let completion_rip = entry.get() + code.len() as u64;
    code.push(0x90);
    code.push(0xf4);

    BoundProducerProgram {
        bytes: code,
        capture_rip,
        completion_rip,
    }
}

fn build_bound_handler(virtual_bar: u64, handler_marker: u8, ack_marker: u8) -> Vec<u8> {
    let mut code = Vec::new();
    emit_debug(&mut code, handler_marker);
    emit_movabs(&mut code, 3, virtual_bar);
    code.extend_from_slice(&[0x8a, 0x83]);
    code.extend_from_slice(&(VIRTIO_ISR_OFFSET as u32).to_le_bytes());
    emit_cmp_al(&mut code, VIRTIO_ISR_QUEUE_INTERRUPT);
    emit_debug(&mut code, ack_marker);
    emit_movabs(&mut code, 3, LAPIC_VIRTUAL_PAGE);
    code.extend_from_slice(&[0xc7, 0x83]);
    code.extend_from_slice(&LAPIC_EOI_OFFSET.to_le_bytes());
    code.extend_from_slice(&0_u32.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xcf]);
    code
}

fn interrupt_gates(
    first_handler: GuestPhysAddr,
    second_handler: GuestPhysAddr,
) -> Vec<LongModeInterruptGate> {
    vec![
        LongModeInterruptGate::new(RESTORED_MULTI_PRODUCER_FIRST_VECTOR, first_handler),
        LongModeInterruptGate::new(RESTORED_MULTI_PRODUCER_SECOND_VECTOR, second_handler),
    ]
}

fn multi_producer_registration_pair(
) -> Result<crate::kvm::sys::HostRegistrationSpecPair, Error> {
    crate::kvm::sys::HostRegistrationSpecPair::new([
        crate::kvm::sys::HostRegistrationSpec::new(
            TWO_HOST_REGISTRATION_FIRST_BAR + 0x100,
            2,
            0,
            RESTORED_MULTI_PRODUCER_FIRST_GSI,
        )?,
        crate::kvm::sys::HostRegistrationSpec::new(
            TWO_HOST_REGISTRATION_SECOND_BAR + 0x100,
            2,
            0,
            RESTORED_MULTI_PRODUCER_SECOND_GSI,
        )?,
    ])
}

fn expected_multi_producer_ioapic_entries() -> [u64; 2] {
    [
        u64::from(RESTORED_MULTI_PRODUCER_FIRST_VECTOR),
        u64::from(RESTORED_MULTI_PRODUCER_SECOND_VECTOR)
            | (u64::from(TWO_VCPU_CHECKPOINT_SECOND_ID.get()) << IOAPIC_DESTINATION_SHIFT),
    ]
}

fn configure_multi_producer_ioapic(vm: &crate::kvm::Vm) -> Result<(), Error> {
    let snapshot = vm.capture_ioapic_state()?;
    if snapshot.irr() != 0 {
        return Err(page_set_error(
            "restored multi-producer initial IOAPIC quiescence",
            format!("expected zero IOAPIC IRR, got {:#x}", snapshot.irr()),
        ));
    }
    let expected = expected_multi_producer_ioapic_entries();
    let configured = snapshot
        .with_redirection_entry(RESTORED_MULTI_PRODUCER_FIRST_GSI as usize, expected[0])
        .and_then(|state| {
            state.with_redirection_entry(
                RESTORED_MULTI_PRODUCER_SECOND_GSI as usize,
                expected[1],
            )
        })
        .expect("fixed multi-producer IOAPIC pins remain valid");
    vm.restore_ioapic_state(&configured)?;
    let readback = multi_producer_ioapic_entries(vm)?;
    if readback != expected {
        return Err(page_set_error(
            "restored multi-producer IOAPIC configuration",
            format!("expected {expected:?}, got {readback:?}"),
        ));
    }
    Ok(())
}

fn multi_producer_ioapic_entries(vm: &crate::kvm::Vm) -> Result<[u64; 2], Error> {
    let snapshot = vm.capture_ioapic_state()?;
    Ok([
        snapshot
            .redirection_entry(RESTORED_MULTI_PRODUCER_FIRST_GSI as usize)
            .expect("fixed first multi-producer IOAPIC pin remains valid"),
        snapshot
            .redirection_entry(RESTORED_MULTI_PRODUCER_SECOND_GSI as usize)
            .expect("fixed second multi-producer IOAPIC pin remains valid"),
    ])
}

fn corrupt_multi_producer_pages(vm: &mut crate::kvm::Vm) -> Result<(), Error> {
    let memory = vm.guest_memory_mut().ok_or_else(|| {
        page_set_error(
            "restored multi-producer page corruption",
            "VM lost registered guest memory",
        )
    })?;
    for (index, address) in MULTI_PRODUCER_OWNERSHIP_SET.iter().enumerate() {
        memory.write(
            *address,
            &vec![0xa5_u8.wrapping_add(index as u8); LONG_MODE_PAGE_SIZE as usize],
        )?;
    }
    Ok(())
}

fn require_multi_producer_mutation(
    mutation: &BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison,
) -> Result<(), Error> {
    two_vcpu_two_device_require_full_mismatch(mutation)?;
    for page in [
        TRANSACTION_COUPLED_FIRST_PAGE,
        TRANSACTION_COUPLED_SECOND_PAGE,
    ] {
        if mutation.controller().page_exact(page) != Some(false) {
            return Err(page_set_error(
                "restored multi-producer queue-page mutation",
                format!("queue page {:#x} did not mismatch", page.get()),
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_bound_producer(
    vcpu: &mut Vcpu,
    index: usize,
    vm: &mut crate::kvm::Vm,
    mmio: &mut MmioBus,
    registrations: &crate::kvm::sys::ReconstructedHostRegistrationPair,
    payloads: &[[u8; crate::portio::pci::virtio_blk::VIRTIO_BLK_SECTOR_SIZE]; 2],
    doorbell_events: &mut [u64; 2],
    irqfd_signals: &mut [u32; 2],
    completions: &mut [Option<crate::portio::pci::virtio_blk::VirtioBlkQueueCompletion>; 4],
    completion_rip: u64,
) -> Result<Vec<u8>, Error> {
    let (write_notify, read_notify, done, expected_proof) = if index == 0 {
        (
            FIRST_WRITE_NOTIFY,
            FIRST_READ_NOTIFY,
            FIRST_DONE_MARKER,
            RESTORED_MULTI_PRODUCER_FIRST_PROOF.as_slice(),
        )
    } else {
        (
            SECOND_WRITE_NOTIFY,
            SECOND_READ_NOTIFY,
            SECOND_DONE_MARKER,
            RESTORED_MULTI_PRODUCER_SECOND_PROOF.as_slice(),
        )
    };
    let mut port_io = PortIoBus::with_debug_port();

    for _ in 0..PRODUCER_EXIT_BUDGET {
        let exit = vcpu.run_once()?;
        let disposition = dispatch_vcpu_exit(vcpu, exit, &mut port_io, mmio)?;
        match disposition {
            VmExitDisposition::Continue(continuation) => {
                if is_debug_output(&continuation, write_notify) {
                    service_write_readback_notification(
                        index,
                        RequestKind::Write,
                        registrations,
                        vm,
                        mmio,
                        doorbell_events,
                        irqfd_signals,
                        completions,
                        payloads,
                    )?;
                } else if is_debug_output(&continuation, read_notify) {
                    service_write_readback_notification(
                        index,
                        RequestKind::Read,
                        registrations,
                        vm,
                        mmio,
                        doorbell_events,
                        irqfd_signals,
                        completions,
                        payloads,
                    )?;
                } else if is_debug_output(&continuation, done) {
                    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
                    if proof.as_slice() != expected_proof {
                        return Err(page_set_error(
                            "restored multi-producer producer proof",
                            format!(
                                "producer {index} expected {expected_proof:?}, got {proof:?}"
                            ),
                        ));
                    }
                    let first_slot = index * 2;
                    if completions[first_slot].is_none() || completions[first_slot + 1].is_none() {
                        return Err(page_set_error(
                            "restored multi-producer completion ownership",
                            format!("producer {index} did not complete both requests"),
                        ));
                    }
                    if doorbell_events[index] != 2 || irqfd_signals[index] != 2 {
                        return Err(page_set_error(
                            "restored multi-producer per-producer counts",
                            format!(
                                "producer {index} expected two doorbells/irqfd signals, got {} / {}",
                                doorbell_events[index], irqfd_signals[index]
                            ),
                        ));
                    }
                    if mmio.take_device_event_record().is_some() {
                        return Err(page_set_error(
                            "restored multi-producer accelerated notify ownership",
                            format!("producer {index} left a userspace device notify event"),
                        ));
                    }
                    let _ = single_step_to_quiescence(
                        vcpu,
                        completion_rip,
                        "restored multi-producer completion quiescence",
                    )?;
                    return Ok(proof);
                }
            }
            VmExitDisposition::Stopped(report) => {
                return Err(page_set_error(
                    "restored multi-producer execution",
                    format!("producer {index} stopped before completion: {report}"),
                ));
            }
        }
    }

    Err(page_set_error(
        "restored multi-producer exit budget",
        format!("producer {index} exceeded bounded exit budget"),
    ))
}

#[cfg(test)]
mod restored_multi_producer_tests {
    use super::*;

    #[test]
    fn multi_producer_routes_are_distinct_and_target_the_expected_apic_ids() {
        assert_eq!(
            expected_multi_producer_ioapic_entries(),
            [0x50, 0x0100_0000_0000_0051]
        );
        assert_eq!(MULTI_PRODUCER_OWNERSHIP_SET.len(), 5);
        assert_ne!(
            deterministic_write_readback_sector(),
            second_write_readback_sector()
        );
        assert_eq!(RESTORED_MULTI_PRODUCER_FIRST_PROOF, b"W0aMR0aXD");
        assert_eq!(RESTORED_MULTI_PRODUCER_SECOND_PROOF, b"Y1bNZ1bQE");
    }
}
