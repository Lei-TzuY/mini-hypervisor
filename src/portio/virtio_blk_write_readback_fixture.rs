use super::pci::virtio_blk::VIRTIO_BLK_T_OUT;

pub const VIRTIO_BLK_WRITE_READBACK_PROOF: &[u8; 7] = b"PBWONRD";

const VIRTIO_BLK_WRITE_READBACK_EXIT_BUDGET: u32 = 44;
const WRITE_NOTIFY_BARRIER: u8 = b'W';
const READ_NOTIFY_BARRIER: u8 = b'N';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioBlkWriteReadbackGuestResult {
    io_exits: Vec<PortIoExit>,
    mmio_exits: Vec<MmioExit>,
    proof: Vec<u8>,
    write_completion: VirtioBlkQueueCompletion,
    read_completion: VirtioBlkQueueCompletion,
    backing: Vec<u8>,
    readback: Vec<u8>,
    request_status: u8,
    used_idx: u16,
    first_used_id: u32,
    first_used_len: u32,
    second_used_id: u32,
    second_used_len: u32,
    report: VmExitReport,
}

impl VirtioBlkWriteReadbackGuestResult {
    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn mmio_exits(&self) -> &[MmioExit] {
        &self.mmio_exits
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn write_completion(&self) -> VirtioBlkQueueCompletion {
        self.write_completion
    }

    #[must_use]
    pub const fn read_completion(&self) -> VirtioBlkQueueCompletion {
        self.read_completion
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
    pub const fn request_status(&self) -> u8 {
        self.request_status
    }

    #[must_use]
    pub const fn used_idx(&self) -> u16 {
        self.used_idx
    }

    #[must_use]
    pub const fn first_used_id(&self) -> u32 {
        self.first_used_id
    }

    #[must_use]
    pub const fn first_used_len(&self) -> u32 {
        self.first_used_len
    }

    #[must_use]
    pub const fn second_used_id(&self) -> u32 {
        self.second_used_id
    }

    #[must_use]
    pub const fn second_used_len(&self) -> u32 {
        self.second_used_len
    }

    #[must_use]
    pub const fn report(&self) -> VmExitReport {
        self.report
    }
}

#[must_use]
pub fn deterministic_write_readback_sector() -> [u8; VIRTIO_BLK_SECTOR_SIZE] {
    let mut bytes = [0_u8; VIRTIO_BLK_SECTOR_SIZE];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(29).wrapping_add(7);
    }
    bytes[..16].copy_from_slice(b"BLK-WRITE-0000!!");
    bytes[VIRTIO_BLK_SECTOR_SIZE - 8..].copy_from_slice(b"WRTBACK!");
    bytes
}

pub fn run_virtio_blk_write_readback_guest(
    config: VmConfig,
) -> Result<VirtioBlkWriteReadbackGuestResult, Error> {
    let guest_bytes = build_write_readback_guest();
    let terminal_rip = LONG_MODE_MMIO_GUEST_ENTRY.get()
        + u64::try_from(guest_bytes.len()).expect("fixed guest length fits u64");
    let image = FlatGuestImage::new(
        LONG_MODE_MMIO_GUEST_ENTRY,
        LONG_MODE_MMIO_GUEST_ENTRY,
        &guest_bytes,
    )?;

    let backend = KvmBackend::open()?;
    let mut vm = backend.create_vm()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout =
        LongModeMmioBootLayout::new(memory.region(), image.entry(), LONG_MODE_MMIO_STACK_POINTER)
            .expect("fixed virtio-blk write/readback BAR mapping remains valid");
    layout.install_page_tables(&mut memory)?;
    image.load(&mut memory)?;
    let write_sector = deterministic_write_readback_sector();
    memory.write(GuestPhysAddr::new(VIRTIO_BLK_DATA_GPA), &write_sector)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode(layout.boot_layout())?;

    let pci =
        PciConfigMechanism1::with_virtio_blk(VirtioBlkPciFunction::new(VIRTIO_BLK_BAR0_GPA as u32));
    let mut port_io = PortIoBus::with_debug_port_and_pci_config(pci);
    let mut mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(VIRTIO_BLK_BAR0_GPA)
        .expect("fixed virtio-blk BAR does not overlap another MMIO device");
    if mmio.virtio_blk_sector_at(VIRTIO_BLK_BAR0_GPA) != Some(&deterministic_sector()) {
        return Err(write_readback_verification_error(
            "virtio-blk backing did not start from the integrated deterministic sector",
        ));
    }

    let mut write_completion = None;
    let mut read_completion = None;
    let mut first_backing = None;
    let execution = run_vcpu_until_stopped_with_mmio_observer(
        &mut vcpu,
        &mut port_io,
        &mut mmio,
        VIRTIO_BLK_WRITE_READBACK_EXIT_BUDGET,
        |continuation, mmio| {
            let Some(barrier) = write_readback_barrier(continuation) else {
                return Ok(());
            };
            let event = mmio.take_device_event_record().ok_or_else(|| {
                write_readback_verification_error(format!(
                    "virtio-blk {barrier:#x} barrier arrived without a pending queue event"
                ))
            })?;
            if event
                != MmioDeviceEventRecord::new(
                    VIRTIO_BLK_BAR0_GPA,
                    MmioDeviceEvent::VirtioQueueNotified {
                        queue: VIRTIO_QUEUE_INDEX,
                    },
                )
            {
                return Err(write_readback_verification_error(format!(
                    "unexpected virtio-blk event at barrier {barrier:#x}: {event:?}"
                )));
            }
            let memory = vm.guest_memory_mut().ok_or_else(|| {
                write_readback_verification_error("virtio-blk write/readback VM lost guest memory")
            })?;
            let completion = mmio
                .process_virtio_blk_notification_atomic(VIRTIO_BLK_BAR0_GPA, memory)
                .map_err(|error| {
                    write_readback_verification_error(format!(
                        "virtio-blk atomic queue processing failed: {error}"
                    ))
                })?
                .ok_or_else(|| {
                    write_readback_verification_error(
                        "virtio-blk BAR disappeared before atomic queue processing",
                    )
                })?;

            match barrier {
                WRITE_NOTIFY_BARRIER => {
                    if write_completion.is_some() || read_completion.is_some() {
                        return Err(write_readback_verification_error(
                            "duplicate or out-of-order virtio-blk write barrier",
                        ));
                    }
                    let backing = mmio
                        .virtio_blk_sector_at(VIRTIO_BLK_BAR0_GPA)
                        .ok_or_else(|| {
                            write_readback_verification_error(
                                "virtio-blk backing unavailable after T_OUT",
                            )
                        })?
                        .to_vec();
                    if backing.as_slice() != write_sector {
                        return Err(write_readback_verification_error(
                            "T_OUT completion did not mutate the in-memory backing sector",
                        ));
                    }
                    first_backing = Some(backing);
                    write_completion = Some(completion);
                }
                READ_NOTIFY_BARRIER => {
                    if write_completion.is_none() || read_completion.is_some() {
                        return Err(write_readback_verification_error(
                            "virtio-blk read barrier arrived before exactly one write completion",
                        ));
                    }
                    read_completion = Some(completion);
                }
                _ => unreachable!(),
            }
            Ok(())
        },
    )?;

    let write_completion = write_completion.ok_or_else(|| {
        write_readback_verification_error("virtio-blk T_OUT request was never processed")
    })?;
    let read_completion = read_completion.ok_or_else(|| {
        write_readback_verification_error("virtio-blk T_IN readback was never processed")
    })?;
    if mmio.take_device_event_record().is_some() {
        return Err(write_readback_verification_error(
            "virtio-blk write/readback left an unconsumed device event",
        ));
    }

    validate_write_readback_io(execution.io_exits())?;
    validate_write_readback_mmio(execution.mmio_exits())?;

    let backing = mmio
        .virtio_blk_sector_at(VIRTIO_BLK_BAR0_GPA)
        .ok_or_else(|| write_readback_verification_error("virtio-blk backing unavailable"))?
        .to_vec();
    let memory = vm.guest_memory().ok_or_else(|| {
        write_readback_verification_error("virtio-blk VM lost guest memory before verification")
    })?;
    let used_idx = read_u16(memory, VIRTIO_BLK_USED_GPA + 2)?;
    let first_used_id = read_u32(memory, VIRTIO_BLK_USED_GPA + 4)?;
    let first_used_len = read_u32(memory, VIRTIO_BLK_USED_GPA + 8)?;
    let second_used_id = read_u32(memory, VIRTIO_BLK_USED_GPA + 12)?;
    let second_used_len = read_u32(memory, VIRTIO_BLK_USED_GPA + 16)?;
    let mut readback = vec![0_u8; VIRTIO_BLK_SECTOR_SIZE];
    memory.read(GuestPhysAddr::new(VIRTIO_BLK_DATA_GPA), &mut readback)?;
    let mut request_status = [0xff_u8];
    memory.read(
        GuestPhysAddr::new(VIRTIO_BLK_STATUS_GPA),
        &mut request_status,
    )?;
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    let report = execution.report();

    if first_backing.as_deref() != Some(write_sector.as_slice())
        || backing.as_slice() != write_sector
        || readback.as_slice() != write_sector
        || write_completion.descriptor_id() != 0
        || write_completion.length() != 1
        || write_completion.sector() != 0
        || read_completion.descriptor_id() != 0
        || read_completion.length() != (VIRTIO_BLK_SECTOR_SIZE + 1) as u32
        || read_completion.sector() != 0
        || used_idx != 2
        || first_used_id != 0
        || first_used_len != 1
        || second_used_id != 0
        || second_used_len != (VIRTIO_BLK_SECTOR_SIZE + 1) as u32
        || request_status[0] != VIRTIO_BLK_S_OK
        || proof.as_slice() != VIRTIO_BLK_WRITE_READBACK_PROOF
    {
        return Err(write_readback_verification_error(format!(
            "virtio-blk write/readback verification failed: write={write_completion:?}, read={read_completion:?}, used=({used_idx},{first_used_id},{first_used_len},{second_used_id},{second_used_len}), request_status={:#x}, proof={proof:?}",
            request_status[0]
        )));
    }

    if report.exit() != VcpuExit::Hlt
        || report.rip() != terminal_rip
        || report.rflags() & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
    {
        return Err(write_readback_verification_error(format!(
            "expected virtio-blk write/readback HLT at RIP {terminal_rip:#x} with architectural RFLAGS bit1 set, got {report}"
        )));
    }

    Ok(VirtioBlkWriteReadbackGuestResult {
        io_exits: execution.io_exits().to_vec(),
        mmio_exits: execution.mmio_exits().to_vec(),
        proof,
        write_completion,
        read_completion,
        backing,
        readback,
        request_status: request_status[0],
        used_idx,
        first_used_id,
        first_used_len,
        second_used_id,
        second_used_len,
        report,
    })
}

fn write_readback_barrier(continuation: &VmExitContinuation) -> Option<u8> {
    match continuation {
        VmExitContinuation::PortIo(io)
            if io.direction() == PortIoDirection::Out
                && io.port() == DEBUG_PORT
                && io.size() == 1
                && io.count() == 1
                && matches!(io.output_data(), [WRITE_NOTIFY_BARRIER] | [READ_NOTIFY_BARRIER]) =>
        {
            Some(io.output_data()[0])
        }
        _ => None,
    }
}

fn validate_write_readback_io(exits: &[PortIoExit]) -> Result<(), Error> {
    let selectors = [0x00, 0x34, 0x40, 0x50, 0x64, 0x74, 0x10].map(config_selector);
    if exits.len() != 14 + VIRTIO_BLK_WRITE_READBACK_PROOF.len() {
        return Err(write_readback_verification_error(format!(
            "expected {} port-I/O exits, got {}",
            14 + VIRTIO_BLK_WRITE_READBACK_PROOF.len(),
            exits.len()
        )));
    }
    for (cycle, selector) in selectors.into_iter().enumerate() {
        let address = &exits[cycle * 2];
        let data = &exits[cycle * 2 + 1];
        if address.direction() != PortIoDirection::Out
            || address.port() != PCI_CONFIG_ADDRESS_PORT
            || address.size() != 4
            || address.count() != 1
            || address.output_data() != selector.to_le_bytes()
            || data.direction() != PortIoDirection::In
            || data.port() != PCI_CONFIG_DATA_PORT
            || data.size() != 4
            || data.count() != 1
            || !data.output_data().is_empty()
        {
            return Err(write_readback_verification_error(format!(
                "virtio-blk write/readback PCI cycle {cycle} mismatch"
            )));
        }
    }
    for (exit, expected) in exits[14..]
        .iter()
        .zip(VIRTIO_BLK_WRITE_READBACK_PROOF.iter().copied())
    {
        if exit.direction() != PortIoDirection::Out
            || exit.port() != DEBUG_PORT
            || exit.size() != 1
            || exit.count() != 1
            || exit.output_data() != [expected]
        {
            return Err(write_readback_verification_error(format!(
                "virtio-blk write/readback proof mismatch at byte {expected:#x}"
            )));
        }
    }
    Ok(())
}

fn validate_write_readback_mmio(exits: &[MmioExit]) -> Result<(), Error> {
    let expected: [(u64, MmioDirection, u32, &[u8]); 22] = [
        (0x300, MmioDirection::Read, 8, &[]),
        (0x14, MmioDirection::Write, 1, &[VIRTIO_STATUS_ACKNOWLEDGE]),
        (
            0x14,
            MmioDirection::Write,
            1,
            &[VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER],
        ),
        (0x00, MmioDirection::Write, 4, &1_u32.to_le_bytes()),
        (0x04, MmioDirection::Read, 4, &[]),
        (0x08, MmioDirection::Write, 4, &1_u32.to_le_bytes()),
        (0x0c, MmioDirection::Write, 4, &1_u32.to_le_bytes()),
        (
            0x14,
            MmioDirection::Write,
            1,
            &[VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK],
        ),
        (0x16, MmioDirection::Write, 2, &0_u16.to_le_bytes()),
        (0x18, MmioDirection::Write, 2, &VIRTIO_QUEUE_SIZE.to_le_bytes()),
        (
            0x20,
            MmioDirection::Write,
            4,
            &(VIRTIO_BLK_DESCRIPTOR_GPA as u32).to_le_bytes(),
        ),
        (0x24, MmioDirection::Write, 4, &0_u32.to_le_bytes()),
        (
            0x28,
            MmioDirection::Write,
            4,
            &(VIRTIO_BLK_AVAIL_GPA as u32).to_le_bytes(),
        ),
        (0x2c, MmioDirection::Write, 4, &0_u32.to_le_bytes()),
        (
            0x30,
            MmioDirection::Write,
            4,
            &(VIRTIO_BLK_USED_GPA as u32).to_le_bytes(),
        ),
        (0x34, MmioDirection::Write, 4, &0_u32.to_le_bytes()),
        (0x1c, MmioDirection::Write, 2, &1_u16.to_le_bytes()),
        (
            0x14,
            MmioDirection::Write,
            1,
            &[VIRTIO_STATUS_ACKNOWLEDGE
                | VIRTIO_STATUS_DRIVER
                | VIRTIO_STATUS_FEATURES_OK
                | VIRTIO_STATUS_DRIVER_OK],
        ),
        (0x14, MmioDirection::Read, 1, &[]),
        (0x100, MmioDirection::Write, 2, &0_u16.to_le_bytes()),
        (0x100, MmioDirection::Write, 2, &0_u16.to_le_bytes()),
        (VIRTIO_ISR_OFFSET, MmioDirection::Read, 1, &[]),
    ];
    if exits.len() != expected.len() {
        return Err(write_readback_verification_error(format!(
            "expected {} virtio-blk write/readback MMIO exits, got {}",
            expected.len(),
            exits.len()
        )));
    }
    for (index, (exit, (offset, direction, length, payload))) in
        exits.iter().zip(expected).enumerate()
    {
        if exit.address() != VIRTIO_BLK_BAR0_GPA + offset
            || exit.direction() != direction
            || exit.length() != length
            || exit.write_data() != payload
        {
            return Err(write_readback_verification_error(format!(
                "virtio-blk write/readback MMIO exit {index} mismatch: {exit:?}"
            )));
        }
    }
    Ok(())
}

fn write_readback_verification_error(detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation: "virtio-blk write/readback proof",
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

fn build_write_readback_guest() -> Vec<u8> {
    let mut code = Vec::new();

    emit_pci_read(&mut code, 0x00);
    emit_cmp_eax(
        &mut code,
        (u32::from(VIRTIO_BLK_PCI_DEVICE_ID) << 16) | u32::from(VIRTIO_PCI_VENDOR_ID),
    );
    emit_pci_read(&mut code, 0x34);
    emit_cmp_eax(&mut code, 0x40);
    emit_pci_read(&mut code, 0x40);
    emit_cmp_eax(&mut code, 0x0110_5009);
    emit_pci_read(&mut code, 0x50);
    emit_cmp_eax(&mut code, 0x0214_6409);
    emit_pci_read(&mut code, 0x64);
    emit_cmp_eax(&mut code, 0x0310_7409);
    emit_pci_read(&mut code, 0x74);
    emit_cmp_eax(&mut code, 0x0410_0009);
    emit_pci_read(&mut code, 0x10);
    code.extend_from_slice(&[0x25, 0xf0, 0xff, 0xff, 0xff]);
    emit_cmp_eax(&mut code, VIRTIO_BLK_BAR0_GPA as u32);
    emit_debug(&mut code, b'P');

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
    emit_mmio_word_write(&mut code, 0x18, VIRTIO_QUEUE_SIZE);
    emit_mmio_dword_write(&mut code, 0x20, VIRTIO_BLK_DESCRIPTOR_GPA as u32);
    emit_mmio_dword_write(&mut code, 0x24, 0);
    emit_mmio_dword_write(&mut code, 0x28, VIRTIO_BLK_AVAIL_GPA as u32);
    emit_mmio_dword_write(&mut code, 0x2c, 0);
    emit_mmio_dword_write(&mut code, 0x30, VIRTIO_BLK_USED_GPA as u32);
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
    emit_debug(&mut code, b'B');

    emit_write_request_setup(&mut code);
    code.extend_from_slice(&[0x66, 0xc7, 0x83, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);
    emit_debug(&mut code, WRITE_NOTIFY_BARRIER);
    emit_first_completion_checks(&mut code);
    emit_debug(&mut code, b'O');

    emit_readback_request_setup(&mut code);
    code.extend_from_slice(&[0x66, 0xc7, 0x83, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);
    emit_debug(&mut code, READ_NOTIFY_BARRIER);
    emit_second_completion_checks(&mut code);
    code.extend_from_slice(&[0x8a, 0x83, 0x00, 0x02, 0x00, 0x00]);
    emit_cmp_al(&mut code, 1);
    emit_debug(&mut code, b'R');
    emit_debug(&mut code, b'D');
    code.push(0xf4);
    code
}

fn emit_write_request_setup(code: &mut Vec<u8>) {
    emit_request_descriptors(code, VIRTQ_DESC_F_NEXT);
    emit_request_header(code, VIRTIO_BLK_T_OUT);
    emit_status_sentinel(code);
    emit_movabs(code, 7, VIRTIO_BLK_AVAIL_GPA);
    code.extend_from_slice(&[0xc7, 0x07, 0x00, 0x00, 0x01, 0x00]);
    code.extend_from_slice(&[0xc7, 0x47, 0x04, 0x00, 0x00, 0x00, 0x00]);
    emit_zero_used_ring(code);
}

fn emit_readback_request_setup(code: &mut Vec<u8>) {
    emit_request_descriptors(code, VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE);
    emit_request_header(code, VIRTIO_BLK_T_IN);
    emit_status_sentinel(code);

    emit_movabs(code, 7, VIRTIO_BLK_DATA_GPA);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&0x5a5a_5a5a_5a5a_5a5a_u64.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xc7, 0xc1, 0x40, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xf3, 0x48, 0xab]);

    emit_movabs(code, 7, VIRTIO_BLK_AVAIL_GPA);
    code.extend_from_slice(&[0xc7, 0x07, 0x00, 0x00, 0x02, 0x00]);
    code.extend_from_slice(&[0x66, 0xc7, 0x47, 0x06, 0x00, 0x00]);
}

fn emit_request_descriptors(code: &mut Vec<u8>, data_flags: u16) {
    emit_movabs(code, 7, VIRTIO_BLK_DESCRIPTOR_GPA);
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

fn emit_request_header(code: &mut Vec<u8>, request_type: u32) {
    emit_movabs(code, 7, VIRTIO_BLK_HEADER_GPA);
    code.extend_from_slice(&[0xc7, 0x07]);
    code.extend_from_slice(&request_type.to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x04, 0x00, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x08, 0x00, 0x00, 0x00, 0x00]);
}

fn emit_status_sentinel(code: &mut Vec<u8>) {
    emit_movabs(code, 7, VIRTIO_BLK_STATUS_GPA);
    code.extend_from_slice(&[0xc6, 0x07, 0xff]);
}

fn emit_zero_used_ring(code: &mut Vec<u8>) {
    emit_movabs(code, 7, VIRTIO_BLK_USED_GPA);
    code.extend_from_slice(&[0x48, 0xc7, 0x07, 0x00, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x08, 0x00, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x10, 0x00, 0x00, 0x00, 0x00]);
}

fn emit_first_completion_checks(code: &mut Vec<u8>) {
    emit_movabs(code, 7, VIRTIO_BLK_USED_GPA);
    code.extend_from_slice(&[0x0f, 0xb7, 0x47, 0x02, 0x83, 0xf8, 0x01]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x04, 0x85, 0xc0]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x08]);
    emit_cmp_eax(code, 1);
    emit_movabs(code, 7, VIRTIO_BLK_STATUS_GPA);
    code.extend_from_slice(&[0x8a, 0x07]);
    emit_cmp_al(code, VIRTIO_BLK_S_OK);
}

fn emit_second_completion_checks(code: &mut Vec<u8>) {
    emit_movabs(code, 7, VIRTIO_BLK_USED_GPA);
    code.extend_from_slice(&[0x0f, 0xb7, 0x47, 0x02, 0x83, 0xf8, 0x02]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x0c, 0x85, 0xc0]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x10]);
    emit_cmp_eax(code, (VIRTIO_BLK_SECTOR_SIZE + 1) as u32);
    emit_movabs(code, 7, VIRTIO_BLK_STATUS_GPA);
    code.extend_from_slice(&[0x8a, 0x07]);
    emit_cmp_al(code, VIRTIO_BLK_S_OK);

    emit_movabs(code, 7, VIRTIO_BLK_DATA_GPA);
    code.extend_from_slice(&[0x48, 0x8b, 0x07]);
    emit_movabs(code, 1, u64::from_le_bytes(*b"BLK-WRIT"));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x48, 0x8b, 0x47, 0x08]);
    emit_movabs(code, 1, u64::from_le_bytes(*b"E-0000!!"));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x48, 0x8b, 0x87, 0xf8, 0x01, 0x00, 0x00]);
    emit_movabs(code, 1, u64::from_le_bytes(*b"WRTBACK!"));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    emit_equal_or_ud2(code);
}

#[cfg(test)]
mod write_readback_fixture_tests {
    use super::*;

    #[test]
    fn write_readback_sector_is_distinct_and_stable() {
        let sector = deterministic_write_readback_sector();
        assert_ne!(sector, deterministic_sector());
        assert_eq!(&sector[..16], b"BLK-WRITE-0000!!");
        assert_eq!(&sector[VIRTIO_BLK_SECTOR_SIZE - 8..], b"WRTBACK!");
    }

    #[test]
    fn write_readback_guest_contains_two_notify_writes_and_terminal_hlt() {
        let bytes = build_write_readback_guest();
        assert_eq!(bytes.last(), Some(&0xf4));
        assert_eq!(
            bytes
                .windows(7)
                .filter(|window| *window == [0x66, 0xc7, 0x83, 0x00, 0x01, 0x00, 0x00])
                .count(),
            2
        );
        for marker in *VIRTIO_BLK_WRITE_READBACK_PROOF {
            assert!(bytes
                .windows(4)
                .any(|window| window == [0xb0, marker, 0xe6, 0xe9]));
        }
    }

    #[test]
    fn exit_budget_matches_config_mmio_proof_and_hlt() {
        assert_eq!(VIRTIO_BLK_WRITE_READBACK_EXIT_BUDGET, 14 + 22 + 7 + 1);
    }
}
