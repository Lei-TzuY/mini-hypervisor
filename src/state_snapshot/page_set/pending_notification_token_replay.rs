pub const PENDING_NOTIFICATION_TOKEN_PROOF: &[u8; 9] = b"W0aMR0aXD";

const PENDING_NOTIFICATION_EXIT_BUDGET: u32 = 32;
const PENDING_NOTIFICATION_WAIT_MILLIS: i32 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingNotificationTokenReplayResult {
    mutation: BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison,
    restored: BoundedTwoVcpuFullControllerTwoVirtioBlkCheckpointComparison,
    schema_version: u16,
    encoded_len: usize,
    page_count: usize,
    bars: [u64; 2],
    token_bar: u64,
    token_queue: u16,
    token_indices: [u16; 2],
    ordinary_capture_rejected: bool,
    capture_pending: [bool; 2],
    reconstructed_pending: [bool; 2],
    queue_indices_at_token: [[u16; 2]; 2],
    queue_indices_after_restore: [[u16; 2]; 2],
    restored_notification_pending: bool,
    backing_unchanged_at_token: bool,
    final_queue_indices: [[u16; 2]; 2],
    doorbell_events: [u64; 2],
    irqfd_signals: [u32; 2],
    proof: Vec<u8>,
    capture_rips: [u64; 2],
    pending_rip: u64,
    completion_rip: u64,
}

impl PendingNotificationTokenReplayResult {
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
    pub const fn bars(&self) -> [u64; 2] {
        self.bars
    }

    #[must_use]
    pub const fn token_bar(&self) -> u64 {
        self.token_bar
    }

    #[must_use]
    pub const fn token_queue(&self) -> u16 {
        self.token_queue
    }

    #[must_use]
    pub const fn token_indices(&self) -> [u16; 2] {
        self.token_indices
    }

    #[must_use]
    pub const fn ordinary_capture_rejected(&self) -> bool {
        self.ordinary_capture_rejected
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
    pub const fn queue_indices_at_token(&self) -> [[u16; 2]; 2] {
        self.queue_indices_at_token
    }

    #[must_use]
    pub const fn queue_indices_after_restore(&self) -> [[u16; 2]; 2] {
        self.queue_indices_after_restore
    }

    #[must_use]
    pub const fn restored_notification_pending(&self) -> bool {
        self.restored_notification_pending
    }

    #[must_use]
    pub const fn backing_unchanged_at_token(&self) -> bool {
        self.backing_unchanged_at_token
    }

    #[must_use]
    pub const fn final_queue_indices(&self) -> [[u16; 2]; 2] {
        self.final_queue_indices
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
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn capture_rips(&self) -> [u64; 2] {
        self.capture_rips
    }

    #[must_use]
    pub const fn pending_rip(&self) -> u64 {
        self.pending_rip
    }

    #[must_use]
    pub const fn completion_rip(&self) -> u64 {
        self.completion_rip
    }
}

pub fn run_pending_notification_token_replay_guest(
) -> Result<PendingNotificationTokenReplayResult, Error> {
    let payloads = [
        deterministic_write_readback_sector(),
        second_write_readback_sector(),
    ];
    let first_program = build_bound_producer_program(0, FIRST_QUEUE, &payloads[0]);
    let second_program = build_bound_producer_program(1, SECOND_QUEUE, &payloads[1]);
    let pending_rip = debug_marker_end_rip(
        &first_program.bytes,
        TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
        FIRST_WRITE_NOTIFY,
        "pending-notification write marker",
    )?;

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
    .expect("fixed pending-notification first interrupt layout remains valid");
    let second_interrupt_layout = LongModeInterruptLayout::with_gates(
        memory.region(),
        second_image.entry(),
        TWO_VCPU_CHECKPOINT_SECOND_STACK,
        interrupt_gates(first_handler.entry(), second_handler.entry()),
    )
    .expect("fixed pending-notification second interrupt layout remains valid");
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
    .expect("fixed pending-notification MMIO mappings remain valid");
    let corrupt_first_layout = LongModeBootLayout::new(
        memory.region(),
        CORRUPT_FIRST_ENTRY,
        CORRUPT_FIRST_STACK,
    )
    .expect("fixed pending-notification first corruption layout remains valid");
    let corrupt_second_layout = LongModeBootLayout::new(
        memory.region(),
        CORRUPT_SECOND_ENTRY,
        CORRUPT_SECOND_STACK,
    )
    .expect("fixed pending-notification second corruption layout remains valid");

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
            "pending-notification MP-state preparation",
            format!("expected RUNNABLE [0, 0], got [{first_mp}, {second_mp}]"),
        ));
    }
    configure_multi_producer_ioapic(&vm)?;

    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty pending-notification MSR policy is valid");
    let mut mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_FIRST_BAR)
        .expect("fixed first pending-notification BAR remains available");
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_SECOND_BAR)
        .expect("fixed second pending-notification BAR remains available");
    let first_ready = ready_device(TWO_HOST_REGISTRATION_FIRST_BAR, FIRST_QUEUE)?;
    let second_ready = ready_device(TWO_HOST_REGISTRATION_SECOND_BAR, SECOND_QUEUE)?;
    mmio.restore_two_virtio_blk_checkpoints_atomic([
        (TWO_HOST_REGISTRATION_FIRST_BAR, &first_ready),
        (TWO_HOST_REGISTRATION_SECOND_BAR, &second_ready),
    ])?;
    require_ready_devices(&mmio)?;

    let pair = multi_producer_registration_pair()?;
    let capture_registrations =
        HostRegistrationPairCheckpoint::capture(pair).reconstruct(&backend, &vm)?;
    let (first_capture_rip, _) = two_vcpu_two_device_retired_barrier(
        &mut first,
        FIRST_CAPTURE_MARKER,
        first_program.capture_rip,
        "first pending-notification capture barrier",
    )?;
    let (second_capture_rip, _) = two_vcpu_two_device_retired_barrier(
        &mut second,
        SECOND_CAPTURE_MARKER,
        second_program.capture_rip,
        "second pending-notification capture barrier",
    )?;

    let capture_pending = capture_registrations.pending_doorbells()?;
    if capture_pending != [false, false] {
        return two_vcpu_two_device_cleanup_error(
            capture_registrations,
            &vm,
            page_set_error(
                "pending-notification initial acceleration quiescence",
                format!("expected [false, false], got {capture_pending:?}"),
            ),
        );
    }

    let mut first_port_io = PortIoBus::with_debug_port();
    let first_doorbell_count = capture_first_write_notification_without_service(
        &mut first,
        &mut first_port_io,
        &capture_registrations,
        &mut mmio,
        pending_rip,
    )?;

    let queue_indices_at_token = [
        pending_queue_indices(&mmio, TWO_HOST_REGISTRATION_FIRST_BAR)?,
        pending_queue_indices(&mmio, TWO_HOST_REGISTRATION_SECOND_BAR)?,
    ];
    if queue_indices_at_token != [[0, 0], [0, 0]] {
        return two_vcpu_two_device_cleanup_error(
            capture_registrations,
            &vm,
            page_set_error(
                "pending-notification queue boundary",
                format!("expected [[0, 0], [0, 0]], got {queue_indices_at_token:?}"),
            ),
        );
    }
    let backing_unchanged_at_token = mmio
        .virtio_blk_sector_at(TWO_HOST_REGISTRATION_FIRST_BAR)
        .map(|sector| sector == &crate::portio::pci::virtio_blk::deterministic_sector())
        .unwrap_or(false);
    if !backing_unchanged_at_token {
        return two_vcpu_two_device_cleanup_error(
            capture_registrations,
            &vm,
            page_set_error(
                "pending-notification pre-service backing",
                "backing changed before queue service",
            ),
        );
    }
    if capture_registrations.pending_doorbells()? != [false, false] {
        return two_vcpu_two_device_cleanup_error(
            capture_registrations,
            &vm,
            page_set_error(
                "pending-notification drained doorbell ownership",
                "ioeventfd remained pending after host notification was materialized",
            ),
        );
    }

    let ordinary_capture = TwoVcpuTwoDeviceCheckpointTransaction::capture(
        pending_capture_context(&first, &second, &vm, &msr_policy, &mmio),
        pair,
        &capture_registrations,
    );
    let ordinary_capture_rejected = match ordinary_capture {
        Ok(_) => {
            return two_vcpu_two_device_cleanup_error(
                capture_registrations,
                &vm,
                page_set_error(
                    "pending-notification ordinary transaction gate",
                    "ordinary fully-quiescent capture accepted a pending notification without a token",
                ),
            )
        }
        Err(error) => {
            if !error_chain_contains(
                &error,
                "pending notification requires an explicit service token",
            ) {
                return two_vcpu_two_device_cleanup_error(
                    capture_registrations,
                    &vm,
                    page_set_error(
                        "pending-notification ordinary transaction gate",
                        format!("capture failed for an unexpected reason: {error}"),
                    ),
                );
            }
            true
        }
    };

    let pending_capture = TwoVcpuTwoDeviceCheckpointTransaction::capture_with_pending_notification(
        pending_capture_context(&first, &second, &vm, &msr_policy, &mmio),
        pair,
        &capture_registrations,
        TWO_HOST_REGISTRATION_FIRST_BAR,
    );
    let capture_cleanup = capture_registrations.deassign(&vm);
    let (transaction, token) = match (pending_capture, capture_cleanup) {
        (Ok(captured), Ok(())) => captured,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup_error)) => {
            return Err(page_set_error(
                "pending-notification capture cleanup",
                format!("capture failed: {error}; cleanup also failed: {cleanup_error}"),
            ))
        }
    };

    let token_bar = token.bar0();
    let token_queue = token.queue();
    let token_indices = [token.last_avail_idx(), token.last_used_idx()];
    if token_bar != TWO_HOST_REGISTRATION_FIRST_BAR
        || token_queue != 0
        || token_indices != [0, 0]
    {
        return Err(page_set_error(
            "pending-notification token identity",
            format!(
                "expected first BAR queue0 indices 0/0, got bar={token_bar:#x} queue={token_queue} indices={token_indices:?}"
            ),
        ));
    }

    let schema = VersionedTwoVcpuTwoDeviceTransactionV3::from_checkpoint_pair_and_pending_notification(
        transaction.checkpoint(),
        pair,
        token,
    )
    .map_err(|error| page_set_error("encode pending-notification transaction", error.to_string()))?;
    if schema.version() != 3
        || schema.bars() != [TWO_HOST_REGISTRATION_FIRST_BAR, TWO_HOST_REGISTRATION_SECOND_BAR]
        || schema.page_count() != MULTI_PRODUCER_OWNERSHIP_SET.len()
        || schema.pending_notification().bar0() != TWO_HOST_REGISTRATION_FIRST_BAR
        || schema.pending_notification().last_avail_idx() != 0
        || schema.pending_notification().last_used_idx() != 0
    {
        return Err(page_set_error(
            "pending-notification transaction metadata",
            "V3 metadata did not preserve the bounded pending notification",
        ));
    }
    let encoded = schema
        .encode()
        .map_err(|error| page_set_error("encode pending-notification V3", error.to_string()))?;
    let encoded_len = encoded.len();
    drop(transaction);

    let decoded = VersionedTwoVcpuTwoDeviceTransactionV3::decode(&encoded)
        .map_err(|error| page_set_error("decode pending-notification V3", error.to_string()))?;
    let canonical = decoded
        .encode()
        .map_err(|error| page_set_error("re-encode pending-notification V3", error.to_string()))?;
    if canonical != encoded {
        return Err(page_set_error(
            "pending-notification canonical V3",
            "decoded V3 did not reproduce its canonical byte stream",
        ));
    }
    let schema_version = decoded.version();
    let page_count = decoded.page_count();
    let bars = decoded.bars();
    let (checkpoint, materialized_pair, token) = decoded
        .materialize(backend.host_msr_indices())
        .map_err(|error| page_set_error("materialize pending-notification V3", error.to_string()))?;
    let transaction = TwoVcpuTwoDeviceCheckpointTransaction {
        checkpoint,
        registrations: HostRegistrationPairCheckpoint::capture(materialized_pair),
    };

    corrupt_multi_producer_pages(&mut vm)?;
    first.initialize_long_mode(&corrupt_first_layout)?;
    second.initialize_long_mode(&corrupt_second_layout)?;
    first.restore_multiprocessing_state_raw(MP_STATE_HALTED)?;
    second.restore_multiprocessing_state_raw(MP_STATE_UNINITIALIZED)?;
    two_vcpu_two_device_corrupt_controller(&first, &second, &vm)?;
    mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_FIRST_BAR)
        .expect("fixed first pending-notification corruption BAR remains available");
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_SECOND_BAR)
        .expect("fixed second pending-notification corruption BAR remains available");

    let mutation = transaction.checkpoint().verify(&first, &second, &vm, &mmio)?;
    require_multi_producer_mutation(&mutation)?;

    let (restored, registrations) = transaction.restore_and_reconstruct_with_pending_notification(
        &backend,
        &mut first,
        &mut second,
        &mut vm,
        &mut mmio,
        &token,
    )?;
    if !restored.is_exact_match() {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "pending-notification exact restore",
                "token-aware transaction did not restore exactly",
            ),
        );
    }
    let reconstructed_pending = registrations.pending_doorbells()?;
    if reconstructed_pending != [false, false] {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "pending-notification reconstructed acceleration",
                format!("expected fresh eventfds [false, false], got {reconstructed_pending:?}"),
            ),
        );
    }
    let queue_indices_after_restore = [
        pending_queue_indices(&mmio, TWO_HOST_REGISTRATION_FIRST_BAR)?,
        pending_queue_indices(&mmio, TWO_HOST_REGISTRATION_SECOND_BAR)?,
    ];
    if queue_indices_after_restore != [[0, 0], [0, 0]] {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "pending-notification restored queue boundary",
                format!(
                    "expected [[0, 0], [0, 0]], got {queue_indices_after_restore:?}"
                ),
            ),
        );
    }
    let restored_token = mmio
        .capture_virtio_blk_checkpoint_with_pending_notification_at(
            TWO_HOST_REGISTRATION_FIRST_BAR,
        )?
        .ok_or_else(|| {
            page_set_error(
                "pending-notification restored token observation",
                "first virtio-blk BAR disappeared",
            )
        })?
        .1;
    let restored_notification_pending = restored_token == token;
    if !restored_notification_pending {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "pending-notification restored token observation",
                "restored semantic notification did not match the materialized token",
            ),
        );
    }

    let memory = vm.guest_memory_mut().ok_or_else(|| {
        page_set_error(
            "pending-notification restored queue service",
            "VM lost registered guest memory",
        )
    })?;
    let write_completion = mmio
        .process_virtio_blk_notification_atomic(TWO_HOST_REGISTRATION_FIRST_BAR, memory)
        .map_err(|error| {
            page_set_error(
                "pending-notification restored queue service",
                error.to_string(),
            )
        })?
        .ok_or_else(|| {
            page_set_error(
                "pending-notification restored queue service",
                "first virtio-blk BAR disappeared",
            )
        })?;
    if write_completion.descriptor_id() != 0
        || write_completion.length() != 1
        || write_completion.sector() != 0
    {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "pending-notification restored write completion",
                format!("unexpected write completion {write_completion:?}"),
            ),
        );
    }
    let backing = mmio
        .virtio_blk_sector_at(TWO_HOST_REGISTRATION_FIRST_BAR)
        .ok_or_else(|| page_set_error("pending-notification backing", "first device disappeared"))?;
    if backing.as_slice() != payloads[0] {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "pending-notification restored write backing",
                "restored pending notification did not service the captured write payload",
            ),
        );
    }

    let mut doorbell_events = [first_doorbell_count, 0];
    let mut irqfd_signals = [0_u32; 2];
    let mut completions = [None; 4];
    completions[0] = Some(write_completion);
    registrations.signal_irq(0)?;
    irqfd_signals[0] = 1;

    let replay = resume_first_producer_after_pending_completion(
        &mut first,
        &mut first_port_io,
        &mut vm,
        &mut mmio,
        &registrations,
        &payloads,
        &mut doorbell_events,
        &mut irqfd_signals,
        &mut completions,
        first_program.completion_rip,
    );
    let cleanup = registrations.deassign(&vm);
    let proof = match (replay, cleanup) {
        (Ok(proof), Ok(())) => proof,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup_error)) => {
            return Err(page_set_error(
                "pending-notification replay cleanup",
                format!("replay failed: {error}; cleanup also failed: {cleanup_error}"),
            ))
        }
    };

    let final_queue_indices = [
        pending_queue_indices(&mmio, TWO_HOST_REGISTRATION_FIRST_BAR)?,
        pending_queue_indices(&mmio, TWO_HOST_REGISTRATION_SECOND_BAR)?,
    ];
    if final_queue_indices != [[2, 2], [0, 0]]
        || doorbell_events != [2, 0]
        || irqfd_signals != [2, 0]
    {
        return Err(page_set_error(
            "pending-notification final ownership",
            format!(
                "expected queues [[2,2],[0,0]], doorbells [2,0], irqfd [2,0]; got queues={final_queue_indices:?} doorbells={doorbell_events:?} irqfd={irqfd_signals:?}"
            ),
        ));
    }
    let readback = read_data(&vm, FIRST_QUEUE.data)?;
    let backing = mmio
        .virtio_blk_sector_at(TWO_HOST_REGISTRATION_FIRST_BAR)
        .ok_or_else(|| page_set_error("pending-notification backing", "first device disappeared"))?;
    if readback.as_slice() != payloads[0] || backing.as_slice() != payloads[0] {
        return Err(page_set_error(
            "pending-notification mutable continuity",
            "first device did not preserve write/readback payload across token restore",
        ));
    }

    Ok(PendingNotificationTokenReplayResult {
        mutation,
        restored,
        schema_version,
        encoded_len,
        page_count,
        bars,
        token_bar,
        token_queue,
        token_indices,
        ordinary_capture_rejected,
        capture_pending,
        reconstructed_pending,
        queue_indices_at_token,
        queue_indices_after_restore,
        restored_notification_pending,
        backing_unchanged_at_token,
        final_queue_indices,
        doorbell_events,
        irqfd_signals,
        proof,
        capture_rips: [first_capture_rip, second_capture_rip],
        pending_rip,
        completion_rip: first_program.completion_rip,
    })
}

fn capture_first_write_notification_without_service(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    registrations: &crate::kvm::sys::ReconstructedHostRegistrationPair,
    mmio: &mut MmioBus,
    pending_rip: u64,
) -> Result<u64, Error> {
    for _ in 0..PENDING_NOTIFICATION_EXIT_BUDGET {
        let exit = vcpu.run_once()?;
        let disposition = dispatch_vcpu_exit(vcpu, exit, port_io, mmio)?;
        match disposition {
            VmExitDisposition::Continue(continuation)
                if is_debug_output(&continuation, FIRST_WRITE_NOTIFY) =>
            {
                if mmio.take_device_event_record().is_some() {
                    return Err(page_set_error(
                        "pending-notification accelerated write",
                        "ioeventfd-consumed notify unexpectedly reached userspace MMIO",
                    ));
                }
                let count = registrations.wait_doorbell(0, PENDING_NOTIFICATION_WAIT_MILLIS)?;
                if count != 1 {
                    return Err(page_set_error(
                        "pending-notification accelerated write",
                        format!("expected doorbell count 1, got {count}"),
                    ));
                }
                if !mmio.apply_virtio_blk_host_notification(TWO_HOST_REGISTRATION_FIRST_BAR, 0)? {
                    return Err(page_set_error(
                        "pending-notification accelerated write",
                        "first virtio-blk BAR disappeared before notification materialization",
                    ));
                }
                retire_current_debug_marker(
                    vcpu,
                    pending_rip,
                    "pending-notification pre-service barrier",
                )?;
                return Ok(count);
            }
            VmExitDisposition::Continue(_) => {}
            VmExitDisposition::Stopped(report) => {
                return Err(page_set_error(
                    "pending-notification accelerated write",
                    format!("producer stopped before notification capture: {report}"),
                ))
            }
        }
    }
    Err(page_set_error(
        "pending-notification accelerated write",
        "producer exceeded bounded exit budget before notification capture",
    ))
}

#[cfg(test)]
mod pending_notification_token_tests {
    use super::*;

    #[test]
    fn pending_notification_proof_and_v3_scope_are_stable() {
        let first = build_bound_producer_program(
            0,
            FIRST_QUEUE,
            &deterministic_write_readback_sector(),
        );
        assert!(
            debug_marker_end_rip(
                &first.bytes,
                TWO_VCPU_CHECKPOINT_FIRST_ENTRY,
                FIRST_WRITE_NOTIFY,
                "test pending notification marker",
            )
            .unwrap()
                > first.capture_rip
        );
        assert_eq!(PENDING_NOTIFICATION_TOKEN_PROOF, b"W0aMR0aXD");
        assert_eq!(MULTI_PRODUCER_OWNERSHIP_SET.len(), 5);
    }
}
