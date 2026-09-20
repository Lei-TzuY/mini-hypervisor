pub const PENDING_COMPLETION_TOKEN_PROOF: &[u8; 9] = b"W0aMR0aXD";

const PENDING_COMPLETION_EXIT_BUDGET: u32 = 32;
const PENDING_COMPLETION_WAIT_MILLIS: i32 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCompletionTokenReplayResult {
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
    final_queue_indices: [[u16; 2]; 2],
    doorbell_events: [u64; 2],
    irqfd_signals: [u32; 2],
    proof: Vec<u8>,
    capture_rips: [u64; 2],
    pending_rip: u64,
    completion_rip: u64,
}

impl PendingCompletionTokenReplayResult {
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

pub fn run_pending_completion_token_replay_guest(
) -> Result<PendingCompletionTokenReplayResult, Error> {
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
        "pending-completion write marker",
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
    .expect("fixed pending-completion first interrupt layout remains valid");
    let second_interrupt_layout = LongModeInterruptLayout::with_gates(
        memory.region(),
        second_image.entry(),
        TWO_VCPU_CHECKPOINT_SECOND_STACK,
        interrupt_gates(first_handler.entry(), second_handler.entry()),
    )
    .expect("fixed pending-completion second interrupt layout remains valid");
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
    .expect("fixed pending-completion MMIO mappings remain valid");
    let corrupt_first_layout = LongModeBootLayout::new(
        memory.region(),
        CORRUPT_FIRST_ENTRY,
        CORRUPT_FIRST_STACK,
    )
    .expect("fixed pending-completion first corruption layout remains valid");
    let corrupt_second_layout = LongModeBootLayout::new(
        memory.region(),
        CORRUPT_SECOND_ENTRY,
        CORRUPT_SECOND_STACK,
    )
    .expect("fixed pending-completion second corruption layout remains valid");

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
            "pending-completion MP-state preparation",
            format!("expected RUNNABLE [0, 0], got [{first_mp}, {second_mp}]"),
        ));
    }
    configure_multi_producer_ioapic(&vm)?;

    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty pending-completion MSR policy is valid");
    let mut mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_FIRST_BAR)
        .expect("fixed first pending-completion BAR remains available");
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_SECOND_BAR)
        .expect("fixed second pending-completion BAR remains available");
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
        "first pending-completion capture barrier",
    )?;
    let (second_capture_rip, _) = two_vcpu_two_device_retired_barrier(
        &mut second,
        SECOND_CAPTURE_MARKER,
        second_program.capture_rip,
        "second pending-completion capture barrier",
    )?;

    let capture_pending = capture_registrations.pending_doorbells()?;
    if capture_pending != [false, false] {
        return two_vcpu_two_device_cleanup_error(
            capture_registrations,
            &vm,
            page_set_error(
                "pending-completion initial acceleration quiescence",
                format!("expected [false, false], got {capture_pending:?}"),
            ),
        );
    }

    let mut first_port_io = PortIoBus::with_debug_port();
    let (write_completion, first_doorbell_count) = service_first_write_without_irq(
        &mut first,
        &mut first_port_io,
        &capture_registrations,
        &mut vm,
        &mut mmio,
        &payloads[0],
        pending_rip,
    )?;
    let queue_indices_at_token = [
        queue_indices(&mmio, TWO_HOST_REGISTRATION_FIRST_BAR)?,
        queue_indices(&mmio, TWO_HOST_REGISTRATION_SECOND_BAR)?,
    ];
    if queue_indices_at_token != [[1, 1], [0, 0]] {
        return two_vcpu_two_device_cleanup_error(
            capture_registrations,
            &vm,
            page_set_error(
                "pending-completion queue boundary",
                format!("expected [[1, 1], [0, 0]], got {queue_indices_at_token:?}"),
            ),
        );
    }
    if capture_registrations.pending_doorbells()? != [false, false] {
        return two_vcpu_two_device_cleanup_error(
            capture_registrations,
            &vm,
            page_set_error(
                "pending-completion drained doorbell ownership",
                "ioeventfd remained pending after queue service",
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
                    "pending-completion ordinary transaction gate",
                    "ordinary fully-quiescent capture accepted a serviced completion without a token",
                ),
            )
        }
        Err(error) => {
            if !error_chain_contains(
                &error,
                "pending completion requires an explicit delivery token",
            ) {
                return two_vcpu_two_device_cleanup_error(
                    capture_registrations,
                    &vm,
                    page_set_error(
                        "pending-completion ordinary transaction gate",
                        format!("capture failed for an unexpected reason: {error}"),
                    ),
                );
            }
            true
        }
    };

    let pending_capture = TwoVcpuTwoDeviceCheckpointTransaction::capture_with_pending_completion(
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
                "pending-completion capture cleanup",
                format!("capture failed: {error}; cleanup also failed: {cleanup_error}"),
            ))
        }
    };

    let token_bar = token.bar0();
    let token_queue = token.queue();
    let token_indices = [token.last_avail_idx(), token.last_used_idx()];
    if token_bar != TWO_HOST_REGISTRATION_FIRST_BAR
        || token_queue != 0
        || token_indices != [1, 1]
    {
        return Err(page_set_error(
            "pending-completion token identity",
            format!(
                "expected first BAR queue0 indices 1/1, got bar={token_bar:#x} queue={token_queue} indices={token_indices:?}"
            ),
        ));
    }

    let schema = VersionedTwoVcpuTwoDeviceTransactionV2::from_checkpoint_pair_and_pending_completion(
        transaction.checkpoint(),
        pair,
        token,
    )
    .map_err(|error| page_set_error("encode pending-completion transaction", error.to_string()))?;
    if schema.version() != 2
        || schema.bars() != [TWO_HOST_REGISTRATION_FIRST_BAR, TWO_HOST_REGISTRATION_SECOND_BAR]
        || schema.page_count() != MULTI_PRODUCER_OWNERSHIP_SET.len()
        || schema.pending_completion().bar0() != TWO_HOST_REGISTRATION_FIRST_BAR
        || schema.pending_completion().last_avail_idx() != 1
        || schema.pending_completion().last_used_idx() != 1
    {
        return Err(page_set_error(
            "pending-completion transaction metadata",
            "V2 metadata did not preserve the bounded pending completion",
        ));
    }
    let encoded = schema
        .encode()
        .map_err(|error| page_set_error("encode pending-completion V2", error.to_string()))?;
    let encoded_len = encoded.len();
    drop(transaction);

    let decoded = VersionedTwoVcpuTwoDeviceTransactionV2::decode(&encoded)
        .map_err(|error| page_set_error("decode pending-completion V2", error.to_string()))?;
    let canonical = decoded
        .encode()
        .map_err(|error| page_set_error("re-encode pending-completion V2", error.to_string()))?;
    if canonical != encoded {
        return Err(page_set_error(
            "pending-completion canonical V2",
            "decoded V2 did not reproduce its canonical byte stream",
        ));
    }
    let schema_version = decoded.version();
    let page_count = decoded.page_count();
    let bars = decoded.bars();
    let (checkpoint, materialized_pair, token) = decoded
        .materialize(backend.host_msr_indices())
        .map_err(|error| page_set_error("materialize pending-completion V2", error.to_string()))?;
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
    // The V2 byte stream now owns the serviced completion. Replace the entire live MMIO
    // device set rather than asking the ordinary restore path to overwrite an ISR-pending device.
    // This deliberately destroys the live ISR/backing/queue state and proves token-aware restore
    // reconstructs it from decoded ownership rather than inheriting it from the pre-checkpoint bus.
    mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_FIRST_BAR)
        .expect("fixed first pending-completion corruption BAR remains available");
    mmio.register_virtio_blk_device_at(TWO_HOST_REGISTRATION_SECOND_BAR)
        .expect("fixed second pending-completion corruption BAR remains available");

    let mutation = transaction.checkpoint().verify(&first, &second, &vm, &mmio)?;
    require_multi_producer_mutation(&mutation)?;

    let (restored, registrations) = transaction.restore_and_reconstruct_with_pending_completion(
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
                "pending-completion exact restore",
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
                "pending-completion reconstructed acceleration",
                format!("expected fresh eventfds [false, false], got {reconstructed_pending:?}"),
            ),
        );
    }
    let queue_indices_after_restore = [
        queue_indices(&mmio, TWO_HOST_REGISTRATION_FIRST_BAR)?,
        queue_indices(&mmio, TWO_HOST_REGISTRATION_SECOND_BAR)?,
    ];
    if queue_indices_after_restore != [[1, 1], [0, 0]] {
        return two_vcpu_two_device_cleanup_error(
            registrations,
            &vm,
            page_set_error(
                "pending-completion restored queue boundary",
                format!(
                    "expected [[1, 1], [0, 0]], got {queue_indices_after_restore:?}"
                ),
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
                "pending-completion replay cleanup",
                format!("replay failed: {error}; cleanup also failed: {cleanup_error}"),
            ))
        }
    };

    let final_queue_indices = [
        queue_indices(&mmio, TWO_HOST_REGISTRATION_FIRST_BAR)?,
        queue_indices(&mmio, TWO_HOST_REGISTRATION_SECOND_BAR)?,
    ];
    if final_queue_indices != [[2, 2], [0, 0]]
        || doorbell_events != [2, 0]
        || irqfd_signals != [2, 0]
    {
        return Err(page_set_error(
            "pending-completion final ownership",
            format!(
                "expected queues [[2,2],[0,0]], doorbells [2,0], irqfd [2,0]; got queues={final_queue_indices:?} doorbells={doorbell_events:?} irqfd={irqfd_signals:?}"
            ),
        ));
    }
    let readback = read_data(&vm, FIRST_QUEUE.data)?;
    let backing = mmio
        .virtio_blk_sector_at(TWO_HOST_REGISTRATION_FIRST_BAR)
        .ok_or_else(|| page_set_error("pending-completion backing", "first device disappeared"))?;
    if readback.as_slice() != payloads[0] || backing.as_slice() != payloads[0] {
        return Err(page_set_error(
            "pending-completion mutable continuity",
            "first device did not preserve write/readback payload across token restore",
        ));
    }

    Ok(PendingCompletionTokenReplayResult {
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
        final_queue_indices,
        doorbell_events,
        irqfd_signals,
        proof,
        capture_rips: [first_capture_rip, second_capture_rip],
        pending_rip,
        completion_rip: first_program.completion_rip,
    })
}

fn error_chain_contains(error: &Error, needle: &str) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(source) = current {
        if source.to_string().contains(needle) {
            return true;
        }
        current = source.source();
    }
    false
}

fn pending_capture_context<'a>(
    first: &'a Vcpu,
    second: &'a Vcpu,
    vm: &'a crate::kvm::Vm,
    msr_policy: &'a GuestMsrAccessPolicy,
    mmio: &'a MmioBus,
) -> TwoVcpuTwoDeviceCaptureContext<'a> {
    TwoVcpuTwoDeviceCaptureContext {
        first,
        second,
        vm,
        msr_policy,
        mmio,
        bars: [
            TWO_HOST_REGISTRATION_SECOND_BAR,
            TWO_HOST_REGISTRATION_FIRST_BAR,
        ],
        page_addresses: &MULTI_PRODUCER_OWNERSHIP_SET,
    }
}

fn debug_marker_end_rip(
    bytes: &[u8],
    entry: GuestPhysAddr,
    marker: u8,
    stage: &'static str,
) -> Result<u64, Error> {
    let needle = [0xb0, marker, 0xe6, 0xe9];
    let mut matches = bytes
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle)
        .map(|(index, _)| index);
    let offset = matches
        .next()
        .ok_or_else(|| page_set_error(stage, "debug marker was not present in guest program"))?;
    if matches.next().is_some() {
        return Err(page_set_error(
            stage,
            "debug marker was not unique in guest program",
        ));
    }
    Ok(entry.get() + (offset + needle.len()) as u64)
}

fn retire_current_debug_marker(
    vcpu: &mut Vcpu,
    expected_rip: u64,
    stage: &'static str,
) -> Result<(), Error> {
    vcpu.set_guest_single_step(true)?;
    let step_result = vcpu.run_once();
    let disable_result = vcpu.set_guest_single_step(false);
    let step_exit = match (step_result, disable_result) {
        (Ok(exit), Ok(())) => exit,
        (Err(error), _) | (Ok(_), Err(error)) => return Err(error),
    };
    if step_exit != VcpuExit::Debug {
        return Err(page_set_error(
            stage,
            format!("expected KVM_EXIT_DEBUG after retiring marker, got {step_exit:?}"),
        ));
    }
    let registers = vcpu.registers()?;
    if registers.rip != expected_rip
        || registers.rflags & 0x2 != 0x2
        || registers.rflags & (1 << 9) != 0
    {
        return Err(page_set_error(
            stage,
            format!(
                "expected IF-cleared rip={expected_rip:#x}, got rip={:#x} rflags={:#x}",
                registers.rip, registers.rflags
            ),
        ));
    }
    Ok(())
}

fn service_first_write_without_irq(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    registrations: &crate::kvm::sys::ReconstructedHostRegistrationPair,
    vm: &mut crate::kvm::Vm,
    mmio: &mut MmioBus,
    payload: &[u8; crate::portio::pci::virtio_blk::VIRTIO_BLK_SECTOR_SIZE],
    pending_rip: u64,
) -> Result<(crate::portio::pci::virtio_blk::VirtioBlkQueueCompletion, u64), Error> {
    for _ in 0..PENDING_COMPLETION_EXIT_BUDGET {
        let exit = vcpu.run_once()?;
        let disposition = dispatch_vcpu_exit(vcpu, exit, port_io, mmio)?;
        match disposition {
            VmExitDisposition::Continue(continuation)
                if is_debug_output(&continuation, FIRST_WRITE_NOTIFY) =>
            {
                if mmio.take_device_event_record().is_some() {
                    return Err(page_set_error(
                        "pending-completion accelerated write",
                        "ioeventfd-consumed notify unexpectedly reached userspace MMIO",
                    ));
                }
                let count = registrations.wait_doorbell(0, PENDING_COMPLETION_WAIT_MILLIS)?;
                if count != 1 {
                    return Err(page_set_error(
                        "pending-completion accelerated write",
                        format!("expected doorbell count 1, got {count}"),
                    ));
                }
                if !mmio.apply_virtio_blk_host_notification(TWO_HOST_REGISTRATION_FIRST_BAR, 0)? {
                    return Err(page_set_error(
                        "pending-completion accelerated write",
                        "first virtio-blk BAR disappeared before queue service",
                    ));
                }
                let memory = vm.guest_memory_mut().ok_or_else(|| {
                    page_set_error(
                        "pending-completion accelerated write",
                        "VM lost registered guest memory",
                    )
                })?;
                let completion = mmio
                    .process_virtio_blk_notification_atomic(
                        TWO_HOST_REGISTRATION_FIRST_BAR,
                        memory,
                    )
                    .map_err(|error| {
                        page_set_error(
                            "pending-completion accelerated write",
                            error.to_string(),
                        )
                    })?
                    .ok_or_else(|| {
                        page_set_error(
                            "pending-completion accelerated write",
                            "first virtio-blk BAR disappeared during queue service",
                        )
                    })?;
                if completion.descriptor_id() != 0
                    || completion.length() != 1
                    || completion.sector() != 0
                {
                    return Err(page_set_error(
                        "pending-completion accelerated write",
                        format!("unexpected write completion {completion:?}"),
                    ));
                }
                let backing = mmio
                    .virtio_blk_sector_at(TWO_HOST_REGISTRATION_FIRST_BAR)
                    .ok_or_else(|| {
                        page_set_error(
                            "pending-completion accelerated write",
                            "first backing disappeared",
                        )
                    })?;
                if backing.as_slice() != payload {
                    return Err(page_set_error(
                        "pending-completion accelerated write",
                        "write did not mutate backing before interrupt delivery",
                    ));
                }
                retire_current_debug_marker(
                    vcpu,
                    pending_rip,
                    "pending-completion serviced-without-irq barrier",
                )?;
                return Ok((completion, count));
            }
            VmExitDisposition::Continue(_) => {}
            VmExitDisposition::Stopped(report) => {
                return Err(page_set_error(
                    "pending-completion accelerated write",
                    format!("producer stopped before write service: {report}"),
                ))
            }
        }
    }
    Err(page_set_error(
        "pending-completion accelerated write",
        "producer exceeded bounded exit budget before write service",
    ))
}

#[allow(clippy::too_many_arguments)]
fn resume_first_producer_after_pending_completion(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    vm: &mut crate::kvm::Vm,
    mmio: &mut MmioBus,
    registrations: &crate::kvm::sys::ReconstructedHostRegistrationPair,
    payloads: &[[u8; crate::portio::pci::virtio_blk::VIRTIO_BLK_SECTOR_SIZE]; 2],
    doorbell_events: &mut [u64; 2],
    irqfd_signals: &mut [u32; 2],
    completions: &mut [Option<crate::portio::pci::virtio_blk::VirtioBlkQueueCompletion>; 4],
    completion_rip: u64,
) -> Result<Vec<u8>, Error> {
    for _ in 0..PENDING_COMPLETION_EXIT_BUDGET {
        let exit = vcpu.run_once()?;
        let disposition = dispatch_vcpu_exit(vcpu, exit, port_io, mmio)?;
        match disposition {
            VmExitDisposition::Continue(continuation) => {
                if is_debug_output(&continuation, FIRST_READ_NOTIFY) {
                    service_write_readback_notification(
                        0,
                        RequestKind::Read,
                        registrations,
                        vm,
                        mmio,
                        doorbell_events,
                        irqfd_signals,
                        completions,
                        payloads,
                    )?;
                } else if is_debug_output(&continuation, FIRST_DONE_MARKER) {
                    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
                    if proof.as_slice() != PENDING_COMPLETION_TOKEN_PROOF {
                        return Err(page_set_error(
                            "pending-completion producer proof",
                            format!(
                                "expected {:?}, got {proof:?}",
                                PENDING_COMPLETION_TOKEN_PROOF
                            ),
                        ));
                    }
                    if completions[0].is_none() || completions[1].is_none() {
                        return Err(page_set_error(
                            "pending-completion completion ownership",
                            "write/read completion pair was incomplete",
                        ));
                    }
                    if doorbell_events != &[2, 0] || irqfd_signals != &[2, 0] {
                        return Err(page_set_error(
                            "pending-completion accelerated counts",
                            format!(
                                "expected doorbells/irqfd [2,0], got {doorbell_events:?} / {irqfd_signals:?}"
                            ),
                        ));
                    }
                    let _ = single_step_to_quiescence(
                        vcpu,
                        completion_rip,
                        "pending-completion final producer quiescence",
                    )?;
                    return Ok(proof);
                }
            }
            VmExitDisposition::Stopped(report) => {
                return Err(page_set_error(
                    "pending-completion resumed producer",
                    format!("producer stopped before completion: {report}"),
                ))
            }
        }
    }

    Err(page_set_error(
        "pending-completion resumed producer",
        "producer exceeded bounded exit budget after irqfd reconstruction",
    ))
}

#[cfg(test)]
mod pending_completion_token_tests {
    use super::*;

    #[test]
    fn pending_completion_proof_and_v2_scope_are_stable() {
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
                "test pending marker",
            )
            .unwrap()
                > first.capture_rip
        );
        assert_eq!(PENDING_COMPLETION_TOKEN_PROOF, b"W0aMR0aXD");
        assert_eq!(MULTI_PRODUCER_OWNERSHIP_SET.len(), 5);
    }
}
