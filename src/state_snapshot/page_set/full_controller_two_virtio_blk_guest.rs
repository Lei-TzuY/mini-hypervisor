pub const TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR: u64 = 0x1000_0000;
pub const TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR: u64 =
    TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR + crate::portio::pci::virtio_blk::VIRTIO_BLK_BAR_SIZE as u64;
pub const TWO_VIRTIO_BLK_CHECKPOINT_FIRST_STATUS: u8 = 0x01;
pub const TWO_VIRTIO_BLK_CHECKPOINT_SECOND_STATUS: u8 = 0x03;
pub const TWO_VIRTIO_BLK_CHECKPOINT_PROOF: &[u8; 6] = FULL_CONTROLLER_CHECKPOINT_PROOF;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullControllerTwoVirtioBlkCheckpointGuestResult {
    capture: ControllerCheckpointCapture,
    mutation: BoundedFullControllerTwoVirtioBlkCheckpointComparison,
    restored: BoundedFullControllerTwoVirtioBlkCheckpointComparison,
    captured_bars: [u64; 2],
    captured_statuses: [u8; 2],
    restored_statuses: [u8; 2],
    proof: Vec<u8>,
    completion_rflags: u64,
}

impl FullControllerTwoVirtioBlkCheckpointGuestResult {
    #[must_use]
    pub const fn capture(&self) -> ControllerCheckpointCapture {
        self.capture
    }

    #[must_use]
    pub const fn mutation(&self) -> &BoundedFullControllerTwoVirtioBlkCheckpointComparison {
        &self.mutation
    }

    #[must_use]
    pub const fn restored(&self) -> &BoundedFullControllerTwoVirtioBlkCheckpointComparison {
        &self.restored
    }

    #[must_use]
    pub const fn captured_bars(&self) -> [u64; 2] {
        self.captured_bars
    }

    #[must_use]
    pub const fn captured_statuses(&self) -> [u8; 2] {
        self.captured_statuses
    }

    #[must_use]
    pub const fn restored_statuses(&self) -> [u8; 2] {
        self.restored_statuses
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TwoVirtioBlkCheckpointTransport {
    Direct,
    VersionedV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VersionedFullControllerTwoVirtioBlkEvidence {
    schema_version: u16,
    encoded_len: usize,
    page_count: usize,
    msr_count: usize,
    bars: [u64; 2],
    backing_len_each: usize,
    canonical_roundtrip: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedFullControllerTwoVirtioBlkCheckpointGuestResult {
    checkpoint: FullControllerTwoVirtioBlkCheckpointGuestResult,
    schema_version: u16,
    encoded_len: usize,
    page_count: usize,
    msr_count: usize,
    bars: [u64; 2],
    backing_len_each: usize,
    canonical_roundtrip: bool,
}

impl VersionedFullControllerTwoVirtioBlkCheckpointGuestResult {
    #[must_use]
    pub const fn checkpoint(&self) -> &FullControllerTwoVirtioBlkCheckpointGuestResult {
        &self.checkpoint
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
    pub const fn msr_count(&self) -> usize {
        self.msr_count
    }

    #[must_use]
    pub const fn bars(&self) -> [u64; 2] {
        self.bars
    }

    #[must_use]
    pub const fn backing_len_each(&self) -> usize {
        self.backing_len_each
    }

    #[must_use]
    pub const fn canonical_roundtrip(&self) -> bool {
        self.canonical_roundtrip
    }
}

pub fn run_full_controller_two_virtio_blk_checkpoint_guest(
) -> Result<FullControllerTwoVirtioBlkCheckpointGuestResult, Error> {
    let (result, _) =
        run_full_controller_two_virtio_blk_checkpoint_core(TwoVirtioBlkCheckpointTransport::Direct)?;
    Ok(result)
}

pub fn run_versioned_full_controller_two_virtio_blk_checkpoint_guest(
) -> Result<VersionedFullControllerTwoVirtioBlkCheckpointGuestResult, Error> {
    let (checkpoint, evidence) = run_full_controller_two_virtio_blk_checkpoint_core(
        TwoVirtioBlkCheckpointTransport::VersionedV1,
    )?;
    let evidence = evidence.expect("versioned two-device transport always returns schema evidence");
    Ok(VersionedFullControllerTwoVirtioBlkCheckpointGuestResult {
        checkpoint,
        schema_version: evidence.schema_version,
        encoded_len: evidence.encoded_len,
        page_count: evidence.page_count,
        msr_count: evidence.msr_count,
        bars: evidence.bars,
        backing_len_each: evidence.backing_len_each,
        canonical_roundtrip: evidence.canonical_roundtrip,
    })
}

fn run_full_controller_two_virtio_blk_checkpoint_core(
    transport: TwoVirtioBlkCheckpointTransport,
) -> Result<
    (
        FullControllerTwoVirtioBlkCheckpointGuestResult,
        Option<VersionedFullControllerTwoVirtioBlkEvidence>,
    ),
    Error,
> {
    let guest = crate::loader::FlatGuestImage::new(
        CONTROLLER_CHECKPOINT_ENTRY,
        CONTROLLER_CHECKPOINT_ENTRY,
        &FULL_CONTROLLER_GUEST_BYTES,
    )?;
    let slave_handler = crate::loader::FlatGuestImage::new(
        LONG_MODE_INTERRUPT_HANDLER,
        LONG_MODE_INTERRUPT_HANDLER,
        &FULL_CONTROLLER_SLAVE_HANDLER_BYTES,
    )?;
    let ioapic_handler = crate::loader::FlatGuestImage::new(
        FULL_CONTROLLER_IOAPIC_HANDLER,
        FULL_CONTROLLER_IOAPIC_HANDLER,
        &FULL_CONTROLLER_IOAPIC_HANDLER_BYTES,
    )?;

    let backend = crate::kvm::KvmBackend::open()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(
        GuestPhysAddr::new(0),
        crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE,
    )?;
    let layout = crate::interrupt::LongModeInterruptLayout::with_gates(
        memory.region(),
        guest.entry(),
        LONG_MODE_INTERRUPT_STACK_POINTER,
        vec![
            crate::interrupt::LongModeInterruptGate::new(
                FULL_CONTROLLER_SLAVE_VECTOR,
                slave_handler.entry(),
            ),
            crate::interrupt::LongModeInterruptGate::new(
                FULL_CONTROLLER_IOAPIC_VECTOR,
                ioapic_handler.entry(),
            ),
        ],
    )
    .expect("fixed two-device checkpoint interrupt layout remains valid");
    let corrupt_layout = crate::long_mode::LongModeBootLayout::new(
        memory.region(),
        CONTROLLER_CHECKPOINT_CORRUPT_ENTRY,
        CONTROLLER_CHECKPOINT_CORRUPT_STACK,
    )
    .expect("fixed two-device checkpoint corruption layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    slave_handler.load(&mut memory)?;
    ioapic_handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(&layout)?;
    let _ = vcpu.configure_legacy_pic_extint()?;
    full_controller_configure_ioapic(&vm)?;
    let msr_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty two-device checkpoint MSR policy is valid by construction");

    let mut mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR)
        .expect("fixed first virtio-blk BAR is available");
    mmio.register_virtio_blk_device_at(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR)
        .expect("fixed second virtio-blk BAR is available");

    let first_captured = prepared_device(
        TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR,
        TWO_VIRTIO_BLK_CHECKPOINT_FIRST_STATUS,
    )?;
    let second_captured = prepared_device(
        TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR,
        TWO_VIRTIO_BLK_CHECKPOINT_SECOND_STATUS,
    )?;
    mmio.restore_two_virtio_blk_checkpoints_atomic([
        (TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR, &first_captured),
        (TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR, &second_captured),
    ])?;

    let capture = controller_run_to_quiescent_debug(&mut vcpu)?;
    let captured_checkpoint = BoundedFullControllerTwoVirtioBlkCheckpoint::capture(
        &vcpu,
        &vm,
        &msr_policy,
        &mmio,
        [
            TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR,
            TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR,
        ],
        &[CONTROLLER_CHECKPOINT_PAGE],
    )?;
    full_controller_require_capture_contract(captured_checkpoint.controller())?;
    let (checkpoint, versioned) =
        prepare_two_virtio_blk_checkpoint_transport(captured_checkpoint, &backend, transport)?;
    full_controller_require_capture_contract(checkpoint.controller())?;
    if checkpoint.device_bars()
        != [
            TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR,
            TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR,
        ]
        || checkpoint
            .device(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR)
            .map(crate::portio::pci::virtio_blk::VirtioBlkDevice::status)
            != Some(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_STATUS)
        || checkpoint
            .device(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR)
            .map(crate::portio::pci::virtio_blk::VirtioBlkDevice::status)
            != Some(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_STATUS)
    {
        return Err(page_set_error(
            "two virtio-blk checkpoint capture contract",
            "canonical BAR order or distinct captured device states were not preserved",
        ));
    }

    vm.guest_memory_mut()
        .expect("registered two-device checkpoint memory remains VM-owned")
        .write(
            CONTROLLER_CHECKPOINT_PAGE,
            &vec![0xa5; LONG_MODE_PAGE_SIZE as usize],
        )?;
    vcpu.initialize_long_mode(&corrupt_layout)?;
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
    let corrupt_ioapic = checkpoint
        .controller()
        .ioapic()
        .with_redirection_entry(
            FULL_CONTROLLER_IOAPIC_PIN16,
            checkpoint
                .controller()
                .ioapic()
                .redirection_entry(FULL_CONTROLLER_IOAPIC_PIN16)
                .expect("fixed IOAPIC pin remains in range")
                | FULL_CONTROLLER_IOAPIC_REDIR_MASKED,
        )
        .expect("fixed IOAPIC pin remains in range");
    vm.restore_ioapic_state(&corrupt_ioapic)?;
    let mut corrupt_lapic = checkpoint.controller().lapic().clone();
    let lint0 = controller_read_lapic_register(&corrupt_lapic, APIC_LVT0_OFFSET);
    controller_write_lapic_register(
        &mut corrupt_lapic,
        APIC_LVT0_OFFSET,
        lint0 | APIC_LVT_MASKED,
    );
    vcpu.restore_lapic_checkpoint_state(&corrupt_lapic)?;

    let first_corrupt =
        crate::portio::pci::virtio_blk::VirtioBlkDevice::new(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR);
    let second_corrupt =
        crate::portio::pci::virtio_blk::VirtioBlkDevice::new(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR);
    mmio.restore_two_virtio_blk_checkpoints_atomic([
        (TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR, &first_corrupt),
        (TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR, &second_corrupt),
    ])?;

    let mutation = checkpoint.verify(&vcpu, &vm, &mmio)?;
    full_controller_require_full_mismatch(mutation.controller())?;
    if mutation.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR) != Some(false)
        || mutation.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR) != Some(false)
        || mutation.is_exact_match()
    {
        return Err(page_set_error(
            "two virtio-blk checkpoint corruption proof",
            format!(
                "expected both device states to mismatch, got first={:?} second={:?}",
                mutation.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR),
                mutation.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR)
            ),
        ));
    }

    let restored = checkpoint.restore_and_verify(&vcpu, &mut vm, &mut mmio)?;
    if !restored.is_exact_match()
        || restored.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR) != Some(true)
        || restored.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR) != Some(true)
    {
        return Err(page_set_error(
            "two virtio-blk checkpoint exact restore",
            format!(
                "restore mismatch: controller={} first={:?} second={:?}",
                restored.controller().is_exact_match(),
                restored.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR),
                restored.device_exact(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR)
            ),
        ));
    }

    let restored_statuses = [
        mmio.virtio_blk_status_at(TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR)
            .ok_or_else(|| page_set_error("two-device status readback", "first device disappeared"))?,
        mmio.virtio_blk_status_at(TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR)
            .ok_or_else(|| page_set_error("two-device status readback", "second device disappeared"))?,
    ];

    let mut port_io = PortIoBus::with_debug_port();
    let marker_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        CONTROLLER_CHECKPOINT_MARKER,
        "two-device restored-page barrier",
    )?;
    let slave_armed = vcpu.registers()?;
    controller_require_interrupt_enabled(
        "two-device checkpoint slave-PIC armed state",
        slave_armed.rflags,
    )?;

    vm.pulse_gsi_edge(FULL_CONTROLLER_SLAVE_GSI)?;
    let slave_handler_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'S',
        "two-device checkpoint slave-PIC handler",
    )?;
    let bridge_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'B',
        "two-device checkpoint post-slave barrier",
    )?;
    let ioapic_armed = vcpu.registers()?;
    controller_require_interrupt_enabled(
        "two-device checkpoint IOAPIC armed state",
        ioapic_armed.rflags,
    )?;

    vm.pulse_gsi_edge(FULL_CONTROLLER_IOAPIC_GSI)?;
    let ioapic_handler_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'J',
        "two-device checkpoint IOAPIC handler",
    )?;
    let resumed_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'M',
        "two-device checkpoint resumed main",
    )?;
    let completion_io = controller_run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        b'D',
        "two-device checkpoint completion barrier",
    )?;
    let completion = vcpu.registers()?;
    controller_require_interrupt_enabled(
        "two-device checkpoint completion state",
        completion.rflags,
    )?;

    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    let io_exits = [
        marker_io,
        slave_handler_io,
        bridge_io,
        ioapic_handler_io,
        resumed_io,
        completion_io,
    ];
    if proof.as_slice() != TWO_VIRTIO_BLK_CHECKPOINT_PROOF
        || io_exits.len() != TWO_VIRTIO_BLK_CHECKPOINT_PROOF.len()
    {
        return Err(page_set_error(
            "two virtio-blk checkpoint executable proof",
            format!("expected ASBJMD, got {proof:?}"),
        ));
    }

    Ok((
        FullControllerTwoVirtioBlkCheckpointGuestResult {
            capture,
            mutation,
            restored,
            captured_bars: checkpoint.device_bars(),
            captured_statuses: [
                TWO_VIRTIO_BLK_CHECKPOINT_FIRST_STATUS,
                TWO_VIRTIO_BLK_CHECKPOINT_SECOND_STATUS,
            ],
            restored_statuses,
            proof,
            completion_rflags: completion.rflags,
        },
        versioned,
    ))
}

fn prepare_two_virtio_blk_checkpoint_transport(
    checkpoint: BoundedFullControllerTwoVirtioBlkCheckpoint,
    backend: &crate::kvm::KvmBackend,
    transport: TwoVirtioBlkCheckpointTransport,
) -> Result<
    (
        BoundedFullControllerTwoVirtioBlkCheckpoint,
        Option<VersionedFullControllerTwoVirtioBlkEvidence>,
    ),
    Error,
> {
    match transport {
        TwoVirtioBlkCheckpointTransport::Direct => Ok((checkpoint, None)),
        TwoVirtioBlkCheckpointTransport::VersionedV1 => {
            let schema =
                VersionedFullControllerTwoVirtioBlkCheckpointV1::from_checkpoint(&checkpoint)
                    .map_err(|error| {
                        page_set_error(
                            "versioned two virtio-blk checkpoint capture",
                            error.to_string(),
                        )
                    })?;
            let encoded = schema.encode().map_err(|error| {
                page_set_error(
                    "versioned two virtio-blk checkpoint encode",
                    error.to_string(),
                )
            })?;
            let encoded_len = encoded.len();

            // Prove the byte boundary owns the transport: the process-local checkpoint and
            // encoder-side semantic object are discarded before decode/materialize.
            drop(schema);
            drop(checkpoint);

            let decoded =
                VersionedFullControllerTwoVirtioBlkCheckpointV1::decode(&encoded).map_err(
                    |error| {
                        page_set_error(
                            "versioned two virtio-blk checkpoint decode",
                            error.to_string(),
                        )
                    },
                )?;
            let canonical = decoded.encode().map_err(|error| {
                page_set_error(
                    "versioned two virtio-blk checkpoint canonical re-encode",
                    error.to_string(),
                )
            })?;
            if canonical != encoded {
                return Err(page_set_error(
                    "versioned two virtio-blk checkpoint canonical re-encode",
                    "decoded checkpoint did not reproduce the canonical byte stream",
                ));
            }
            let evidence = VersionedFullControllerTwoVirtioBlkEvidence {
                schema_version: decoded.version(),
                encoded_len,
                page_count: decoded.page_count(),
                msr_count: decoded.msr_count(),
                bars: decoded.device_bars(),
                backing_len_each: decoded.backing_len_each(),
                canonical_roundtrip: true,
            };
            let checkpoint = decoded.materialize(backend.host_msr_indices()).map_err(|error| {
                page_set_error(
                    "versioned two virtio-blk checkpoint materialize",
                    error.to_string(),
                )
            })?;
            Ok((checkpoint, Some(evidence)))
        }
    }
}

fn prepared_device(
    bar: u64,
    status: u8,
) -> Result<crate::portio::pci::virtio_blk::VirtioBlkDevice, Error> {
    let mut device = crate::portio::pci::virtio_blk::VirtioBlkDevice::new(bar);
    device
        .write(0x14, &[0x01])
        .map_err(|error| page_set_error("prepare virtio-blk checkpoint device", error.to_string()))?;
    if status == 0x03 {
        device.write(0x14, &[0x03]).map_err(|error| {
            page_set_error("prepare virtio-blk checkpoint device", error.to_string())
        })?;
    }
    if device.status() != status || !device.checkpoint_quiescent() {
        return Err(page_set_error(
            "prepare virtio-blk checkpoint device",
            format!("prepared BAR {bar:#x} status {}, expected {status}", device.status()),
        ));
    }
    Ok(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_devices_have_distinct_valid_quiescent_states() {
        let first = prepared_device(
            TWO_VIRTIO_BLK_CHECKPOINT_FIRST_BAR,
            TWO_VIRTIO_BLK_CHECKPOINT_FIRST_STATUS,
        )
        .unwrap();
        let second = prepared_device(
            TWO_VIRTIO_BLK_CHECKPOINT_SECOND_BAR,
            TWO_VIRTIO_BLK_CHECKPOINT_SECOND_STATUS,
        )
        .unwrap();
        assert!(first.checkpoint_quiescent());
        assert!(second.checkpoint_quiescent());
        assert_eq!(first.status(), 0x01);
        assert_eq!(second.status(), 0x03);
    }
}
