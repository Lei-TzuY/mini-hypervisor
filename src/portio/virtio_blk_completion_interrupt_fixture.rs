use super::pci::virtio::{
    VIRTIO_F_VERSION_1, VIRTIO_ISR_OFFSET, VIRTIO_ISR_QUEUE_INTERRUPT, VIRTIO_PCI_VENDOR_ID,
    VIRTIO_STATUS_ACKNOWLEDGE, VIRTIO_STATUS_DRIVER, VIRTIO_STATUS_DRIVER_OK,
    VIRTIO_STATUS_FEATURES_OK,
};
use super::pci::virtio_blk::{
    deterministic_sector, VirtioBlkPciFunction, VirtioBlkQueueCompletion,
    VIRTIO_BLK_CAPACITY_SECTORS, VIRTIO_BLK_PCI_DEVICE_ID, VIRTIO_BLK_SECTOR_SIZE, VIRTIO_BLK_S_OK,
    VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE,
};
use super::pci::{
    config_selector, PciConfigMechanism1, PCI_CONFIG_ADDRESS_PORT, PCI_CONFIG_DATA_PORT,
};
use super::{PortIoBus, DEBUG_PORT};
use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError};
use crate::interrupt::{
    LONG_MODE_INTERRUPT_HANDLER, LONG_MODE_INTERRUPT_VECTOR, X86_RFLAGS_INTERRUPT_ENABLE,
};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::mmio::interrupt::LongModeMmioInterruptLayout;
use crate::mmio::long_mode::{
    LONG_MODE_MMIO_DEVICE_GPA, LONG_MODE_MMIO_GUEST_ENTRY, LONG_MODE_MMIO_STACK_POINTER,
};
use crate::mmio::{MmioBus, MmioDeviceEvent, MmioDeviceEventRecord};
use crate::vcpu::{MmioDirection, MmioExit, PortIoDirection, PortIoExit, VcpuId};
use crate::vmexit::{dispatch_vcpu_exit, VmExitContinuation, VmExitDisposition};
use std::io;

pub const VIRTIO_BLK_INTERRUPT_BAR0_GPA: u64 = LONG_MODE_MMIO_DEVICE_GPA;
pub const VIRTIO_BLK_INTERRUPT_DESCRIPTOR_GPA: u64 = 0x0001_8000;
pub const VIRTIO_BLK_INTERRUPT_AVAIL_GPA: u64 = 0x0001_8100;
pub const VIRTIO_BLK_INTERRUPT_USED_GPA: u64 = 0x0001_8200;
pub const VIRTIO_BLK_INTERRUPT_HEADER_GPA: u64 = 0x0001_8300;
pub const VIRTIO_BLK_INTERRUPT_DATA_GPA: u64 = 0x0001_8400;
pub const VIRTIO_BLK_INTERRUPT_STATUS_GPA: u64 = 0x0001_8600;
pub const VIRTIO_BLK_INTERRUPT_PROOF: &[u8; 7] = b"PBNIARD";

const VIRTIO_BLK_INTERRUPT_EXIT_BUDGET: u32 = 43;
const VIRTIO_QUEUE_SIZE: u16 = 4;
const VIRTIO_QUEUE_INDEX: u16 = 0;
const NOTIFY_BARRIER: u8 = b'N';
const ISR_HANDLER_BYTE: u8 = b'I';
const ISR_ACK_BARRIER: u8 = b'A';
const COMPLETION_BARRIER: u8 = b'D';
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioBlkCompletionInterruptGuestResult {
    io_exits: Vec<PortIoExit>,
    mmio_exits: Vec<MmioExit>,
    proof: Vec<u8>,
    completion: VirtioBlkQueueCompletion,
    status: u8,
    driver_features: u64,
    queue_enabled: bool,
    data: Vec<u8>,
    request_status: u8,
    used_idx: u16,
    used_id: u32,
    used_len: u32,
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    assert_count: u32,
    deassert_count: u32,
    completion_rflags: u64,
}

impl VirtioBlkCompletionInterruptGuestResult {
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
    pub const fn completion(&self) -> VirtioBlkQueueCompletion {
        self.completion
    }

    #[must_use]
    pub const fn status(&self) -> u8 {
        self.status
    }

    #[must_use]
    pub const fn driver_features(&self) -> u64 {
        self.driver_features
    }

    #[must_use]
    pub const fn queue_enabled(&self) -> bool {
        self.queue_enabled
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
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
    pub const fn used_id(&self) -> u32 {
        self.used_id
    }

    #[must_use]
    pub const fn used_len(&self) -> u32 {
        self.used_len
    }

    #[must_use]
    pub const fn gsi(&self) -> u32 {
        self.gsi
    }

    #[must_use]
    pub const fn vector(&self) -> u8 {
        self.vector
    }

    #[must_use]
    pub const fn lapic_spiv(&self) -> u32 {
        self.lapic_spiv
    }

    #[must_use]
    pub const fn lapic_lint0(&self) -> u32 {
        self.lapic_lint0
    }

    #[must_use]
    pub const fn assert_count(&self) -> u32 {
        self.assert_count
    }

    #[must_use]
    pub const fn deassert_count(&self) -> u32 {
        self.deassert_count
    }

    #[must_use]
    pub const fn completion_rflags(&self) -> u64 {
        self.completion_rflags
    }
}

pub fn run_virtio_blk_completion_interrupt_guest(
    config: VmConfig,
) -> Result<VirtioBlkCompletionInterruptGuestResult, Error> {
    let guest_bytes = build_guest();
    let guest = FlatGuestImage::new(
        LONG_MODE_MMIO_GUEST_ENTRY,
        LONG_MODE_MMIO_GUEST_ENTRY,
        &guest_bytes,
    )?;
    let handler_bytes = build_interrupt_handler();
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
        LONG_MODE_MMIO_STACK_POINTER,
        LONG_MODE_INTERRUPT_VECTOR,
        handler.entry(),
    )
    .expect("fixed virtio-blk interrupt layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(layout.interrupt_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;

    let pci = PciConfigMechanism1::with_virtio_blk(VirtioBlkPciFunction::new(
        VIRTIO_BLK_INTERRUPT_BAR0_GPA as u32,
    ));
    let mut port_io = PortIoBus::with_debug_port_and_pci_config(pci);
    let mut mmio = MmioBus::empty();
    mmio.register_virtio_blk_device_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA)
        .expect("fixed virtio-blk interrupt BAR does not overlap another MMIO device");

    let mut queue_completion = None;
    let mut line_asserted = false;
    let mut assert_count = 0_u32;
    let mut deassert_count = 0_u32;
    let mut io_exits = Vec::new();
    let mut mmio_exits = Vec::new();
    let mut completed_exits = 0_u32;

    loop {
        if completed_exits >= VIRTIO_BLK_INTERRUPT_EXIT_BUDGET {
            return Err(verification_error(format!(
                "virtio-blk INTx completion exceeded exact exit budget {} before final barrier",
                VIRTIO_BLK_INTERRUPT_EXIT_BUDGET
            )));
        }
        let exit = vcpu.run_once()?;
        completed_exits += 1;
        let disposition = dispatch_vcpu_exit(&mut vcpu, exit, &mut port_io, &mut mmio)?;
        match disposition {
            VmExitDisposition::Continue(continuation) => {
                if is_debug_output(&continuation, NOTIFY_BARRIER) {
                    if queue_completion.is_some() || line_asserted {
                        return Err(verification_error(
                            "duplicate virtio-blk INTx notify barrier",
                        ));
                    }
                    let event = mmio.take_device_event_record().ok_or_else(|| {
                        verification_error(
                            "notify barrier arrived without a pending virtio-blk event",
                        )
                    })?;
                    let expected = MmioDeviceEventRecord::new(
                        VIRTIO_BLK_INTERRUPT_BAR0_GPA,
                        MmioDeviceEvent::VirtioQueueNotified {
                            queue: VIRTIO_QUEUE_INDEX,
                        },
                    );
                    if event != expected {
                        return Err(verification_error(format!(
                            "unexpected virtio-blk event at INTx notify barrier: {event:?}"
                        )));
                    }
                    let memory = vm.guest_memory_mut().ok_or_else(|| {
                        verification_error("virtio-blk INTx VM lost registered guest memory")
                    })?;
                    let completion = mmio
                        .process_virtio_blk_notification(VIRTIO_BLK_INTERRUPT_BAR0_GPA, memory)
                        .map_err(|error| {
                            verification_error(format!(
                                "virtio-blk queue processing failed before INTx delivery: {error}"
                            ))
                        })?
                        .ok_or_else(|| {
                            verification_error(
                                "virtio-blk BAR disappeared before INTx queue processing",
                            )
                        })?;
                    vm.set_gsi_level(KvmBackend::IRQCHIP_GSI, true)?;
                    queue_completion = Some(completion);
                    line_asserted = true;
                    assert_count += 1;
                } else if is_debug_output(&continuation, ISR_ACK_BARRIER) {
                    if queue_completion.is_none() || !line_asserted {
                        return Err(verification_error(
                            "virtio-blk ISR ACK barrier arrived without asserted completion line",
                        ));
                    }
                    vm.set_gsi_level(KvmBackend::IRQCHIP_GSI, false)?;
                    line_asserted = false;
                    deassert_count += 1;
                }

                let completion_barrier = is_debug_output(&continuation, COMPLETION_BARRIER);
                match continuation {
                    VmExitContinuation::PortIo(io) => io_exits.push(io),
                    VmExitContinuation::Mmio(access) => mmio_exits.push(access),
                }
                if completion_barrier {
                    break;
                }
            }
            VmExitDisposition::Stopped(report) => {
                return Err(verification_error(format!(
                    "unexpected terminal vCPU exit before virtio-blk INTx completion barrier: {report}"
                )));
            }
        }
    }

    if line_asserted {
        return Err(verification_error(
            "virtio-blk INTx completion left GSI asserted after ISR acknowledgement",
        ));
    }
    if assert_count != 1 || deassert_count != 1 {
        return Err(verification_error(format!(
            "expected one virtio-blk INTx assert/deassert lifecycle, got assert={assert_count}, deassert={deassert_count}"
        )));
    }
    if mmio.take_device_event_record().is_some() {
        return Err(verification_error(
            "virtio-blk INTx execution left extra device event",
        ));
    }
    if completed_exits != VIRTIO_BLK_INTERRUPT_EXIT_BUDGET {
        return Err(verification_error(format!(
            "expected exactly {} exits through virtio-blk INTx barrier, got {completed_exits}",
            VIRTIO_BLK_INTERRUPT_EXIT_BUDGET
        )));
    }

    let completion = queue_completion
        .ok_or_else(|| verification_error("virtio-blk queue never completed before INTx proof"))?;
    validate_io_sequence(&io_exits)?;
    validate_mmio_sequence(&mmio_exits)?;

    let status = mmio
        .virtio_blk_status_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA)
        .ok_or_else(|| verification_error("virtio-blk INTx status unavailable"))?;
    let driver_features = mmio
        .virtio_blk_driver_features_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA)
        .ok_or_else(|| verification_error("virtio-blk INTx driver features unavailable"))?;
    let queue_enabled = mmio
        .virtio_blk_queue_enabled_at(VIRTIO_BLK_INTERRUPT_BAR0_GPA)
        .ok_or_else(|| verification_error("virtio-blk INTx queue state unavailable"))?;

    let memory = vm.guest_memory().ok_or_else(|| {
        verification_error("virtio-blk INTx VM lost guest memory before verification")
    })?;
    let used_idx = read_u16(memory, VIRTIO_BLK_INTERRUPT_USED_GPA + 2)?;
    let used_id = read_u32(memory, VIRTIO_BLK_INTERRUPT_USED_GPA + 4)?;
    let used_len = read_u32(memory, VIRTIO_BLK_INTERRUPT_USED_GPA + 8)?;
    let mut data = vec![0_u8; VIRTIO_BLK_SECTOR_SIZE];
    memory.read(
        GuestPhysAddr::new(VIRTIO_BLK_INTERRUPT_DATA_GPA),
        &mut data,
    )?;
    let mut request_status = [0xff_u8];
    memory.read(
        GuestPhysAddr::new(VIRTIO_BLK_INTERRUPT_STATUS_GPA),
        &mut request_status,
    )?;

    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    let completion_rflags = vcpu.registers()?.rflags;
    let expected_status = VIRTIO_STATUS_ACKNOWLEDGE
        | VIRTIO_STATUS_DRIVER
        | VIRTIO_STATUS_FEATURES_OK
        | VIRTIO_STATUS_DRIVER_OK;

    if completion.descriptor_id() != 0
        || completion.length() != (VIRTIO_BLK_SECTOR_SIZE + 1) as u32
        || completion.sector() != 0
        || status != expected_status
        || driver_features != VIRTIO_F_VERSION_1
        || !queue_enabled
        || used_idx != 1
        || used_id != 0
        || used_len != (VIRTIO_BLK_SECTOR_SIZE + 1) as u32
        || data.as_slice() != deterministic_sector()
        || request_status[0] != VIRTIO_BLK_S_OK
        || proof.as_slice() != VIRTIO_BLK_INTERRUPT_PROOF
    {
        return Err(verification_error(format!(
            "virtio-blk INTx verification failed: completion={completion:?}, status={status:#x}, features={driver_features:#x}, queue_enabled={queue_enabled}, used=({used_idx},{used_id},{used_len}), request_status={:#x}, proof={proof:?}",
            request_status[0]
        )));
    }

    if completion_rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || completion_rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
    {
        return Err(verification_error(format!(
            "expected virtio-blk INTx completion barrier with bit1 and IF set, got RFLAGS {completion_rflags:#x}"
        )));
    }

    Ok(VirtioBlkCompletionInterruptGuestResult {
        io_exits,
        mmio_exits,
        proof,
        completion,
        status,
        driver_features,
        queue_enabled,
        data,
        request_status: request_status[0],
        used_idx,
        used_id,
        used_len,
        gsi: KvmBackend::IRQCHIP_GSI,
        vector: KvmBackend::IRQCHIP_VECTOR,
        lapic_spiv: lapic.spiv(),
        lapic_lint0: lapic.lint0(),
        assert_count,
        deassert_count,
        completion_rflags,
    })
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

fn validate_io_sequence(exits: &[PortIoExit]) -> Result<(), Error> {
    let selectors = [0x00, 0x34, 0x40, 0x50, 0x64, 0x74, 0x10].map(config_selector);
    if exits.len() != 21 {
        return Err(verification_error(format!(
            "expected 21 virtio-blk INTx port-I/O exits, got {}",
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
            return Err(verification_error(format!(
                "virtio-blk INTx PCI config cycle {cycle} mismatch"
            )));
        }
    }
    for (exit, expected) in exits[14..]
        .iter()
        .zip(VIRTIO_BLK_INTERRUPT_PROOF.iter().copied())
    {
        if exit.direction() != PortIoDirection::Out
            || exit.port() != DEBUG_PORT
            || exit.size() != 1
            || exit.count() != 1
            || exit.output_data() != [expected]
        {
            return Err(verification_error(format!(
                "virtio-blk INTx proof output mismatch for byte {expected:#x}"
            )));
        }
    }
    Ok(())
}

fn validate_mmio_sequence(exits: &[MmioExit]) -> Result<(), Error> {
    let expected: [(u64, MmioDirection, u32, &[u8]); 22] = [
        (0x300, MmioDirection::Read, 8, &[]),
        (
            0x14,
            MmioDirection::Write,
            1,
            &[VIRTIO_STATUS_ACKNOWLEDGE],
        ),
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
        (
            0x18,
            MmioDirection::Write,
            2,
            &VIRTIO_QUEUE_SIZE.to_le_bytes(),
        ),
        (
            0x20,
            MmioDirection::Write,
            4,
            &(VIRTIO_BLK_INTERRUPT_DESCRIPTOR_GPA as u32).to_le_bytes(),
        ),
        (0x24, MmioDirection::Write, 4, &0_u32.to_le_bytes()),
        (
            0x28,
            MmioDirection::Write,
            4,
            &(VIRTIO_BLK_INTERRUPT_AVAIL_GPA as u32).to_le_bytes(),
        ),
        (0x2c, MmioDirection::Write, 4, &0_u32.to_le_bytes()),
        (
            0x30,
            MmioDirection::Write,
            4,
            &(VIRTIO_BLK_INTERRUPT_USED_GPA as u32).to_le_bytes(),
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
        (VIRTIO_ISR_OFFSET, MmioDirection::Read, 1, &[]),
        (VIRTIO_ISR_OFFSET, MmioDirection::Read, 1, &[]),
    ];
    if exits.len() != expected.len() {
        return Err(verification_error(format!(
            "expected {} virtio-blk INTx MMIO exits, got {}",
            expected.len(),
            exits.len()
        )));
    }
    for (index, (exit, (offset, direction, length, payload))) in
        exits.iter().zip(expected).enumerate()
    {
        if exit.address() != VIRTIO_BLK_INTERRUPT_BAR0_GPA + offset
            || exit.direction() != direction
            || exit.length() != length
            || exit.write_data() != payload
        {
            return Err(verification_error(format!(
                "virtio-blk INTx MMIO exit {index} mismatch: {exit:?}"
            )));
        }
    }
    Ok(())
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

fn verification_error(detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation: "virtio-blk INTx completion proof",
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

fn build_guest() -> Vec<u8> {
    let mut code = Vec::new();
    emit_pic_setup(&mut code);
    code.extend_from_slice(&[0xfb, 0x90]);

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
    emit_cmp_eax(&mut code, VIRTIO_BLK_INTERRUPT_BAR0_GPA as u32);
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
    emit_mmio_dword_write(
        &mut code,
        0x20,
        VIRTIO_BLK_INTERRUPT_DESCRIPTOR_GPA as u32,
    );
    emit_mmio_dword_write(&mut code, 0x24, 0);
    emit_mmio_dword_write(
        &mut code,
        0x28,
        VIRTIO_BLK_INTERRUPT_AVAIL_GPA as u32,
    );
    emit_mmio_dword_write(&mut code, 0x2c, 0);
    emit_mmio_dword_write(
        &mut code,
        0x30,
        VIRTIO_BLK_INTERRUPT_USED_GPA as u32,
    );
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

    emit_ring_setup(&mut code);
    code.extend_from_slice(&[0x66, 0xc7, 0x83, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);
    emit_debug(&mut code, NOTIFY_BARRIER);
    emit_guest_completion_checks(&mut code);
    emit_movabs(&mut code, 3, 0x0050_0000);
    code.extend_from_slice(&[0x8a, 0x83]);
    code.extend_from_slice(&(VIRTIO_ISR_OFFSET as u32).to_le_bytes());
    emit_cmp_al(&mut code, 0);
    emit_debug(&mut code, b'R');
    emit_debug(&mut code, COMPLETION_BARRIER);
    code.push(0xf4);
    code
}

fn build_interrupt_handler() -> Vec<u8> {
    let mut code = Vec::new();
    emit_debug(&mut code, ISR_HANDLER_BYTE);
    emit_movabs(&mut code, 3, 0x0050_0000);
    code.extend_from_slice(&[0x8a, 0x83]);
    code.extend_from_slice(&(VIRTIO_ISR_OFFSET as u32).to_le_bytes());
    emit_cmp_al(&mut code, VIRTIO_ISR_QUEUE_INTERRUPT);
    emit_debug(&mut code, ISR_ACK_BARRIER);
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

fn emit_pci_read(code: &mut Vec<u8>, offset: u8) {
    code.extend_from_slice(&[0x66, 0xba, 0xf8, 0x0c]);
    code.push(0xb8);
    code.extend_from_slice(&config_selector(offset).to_le_bytes());
    code.push(0xef);
    code.extend_from_slice(&[0x66, 0xba, 0xfc, 0x0c, 0xed]);
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
    code.extend_from_slice(&u32::from(VIRTQ_DESC_F_WRITE).to_le_bytes());

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
    fn fixture_contains_pic_setup_two_isr_reads_and_completion_barrier() {
        let guest = build_guest();
        let handler = build_interrupt_handler();
        assert!(guest.ends_with(&[0xb0, b'D', 0xe6, 0xe9, 0xf4]));
        for marker in *b"PBNRD" {
            assert!(guest
                .windows(4)
                .any(|window| window == [0xb0, marker, 0xe6, 0xe9]));
        }
        for marker in *b"IA" {
            assert!(handler
                .windows(4)
                .any(|window| window == [0xb0, marker, 0xe6, 0xe9]));
        }
        assert!(handler.ends_with(&[0x48, 0xcf]));
    }

    #[test]
    fn exit_budget_matches_exact_userspace_contract() {
        assert_eq!(VIRTIO_BLK_INTERRUPT_EXIT_BUDGET, 7 * 2 + 22 + 7);
    }
}
