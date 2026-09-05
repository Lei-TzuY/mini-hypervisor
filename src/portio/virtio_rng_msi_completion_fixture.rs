use super::pci::virtio::{
    VirtioRngPciFunction, VirtioRngQueueCompletion, VIRTIO_F_VERSION_1, VIRTIO_ISR_OFFSET,
    VIRTIO_ISR_QUEUE_INTERRUPT, VIRTIO_PCI_VENDOR_ID, VIRTIO_RNG_PCI_DEVICE_ID,
    VIRTIO_RNG_TEST_PAYLOAD, VIRTIO_STATUS_ACKNOWLEDGE, VIRTIO_STATUS_DRIVER,
    VIRTIO_STATUS_DRIVER_OK, VIRTIO_STATUS_FEATURES_OK,
};
use super::pci::{
    config_selector, PciConfigMechanism1, PCI_CONFIG_ADDRESS_PORT, PCI_CONFIG_DATA_PORT,
};
use super::virtio_rng_completion_interrupt_fixture::{
    VIRTIO_RNG_INTERRUPT_AVAIL_GPA, VIRTIO_RNG_INTERRUPT_BAR0_GPA,
    VIRTIO_RNG_INTERRUPT_BUFFER_GPA, VIRTIO_RNG_INTERRUPT_DESCRIPTOR_GPA,
    VIRTIO_RNG_INTERRUPT_USED_GPA,
};
use super::{PortIoBus, DEBUG_PORT};
use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError};
use crate::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use crate::kvm::sys::KvmMsiMessage;
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::mmio::interrupt::LongModeMmioInterruptLayout;
use crate::mmio::long_mode::{LONG_MODE_MMIO_GUEST_ENTRY, LONG_MODE_MMIO_STACK_POINTER};
use crate::mmio::{MmioBus, MmioDeviceEvent, MmioDeviceEventRecord};
use crate::vcpu::{MmioDirection, MmioExit, PortIoDirection, PortIoExit, VcpuId};
use crate::vmexit::{dispatch_vcpu_exit, VmExitContinuation, VmExitDisposition};
use std::io;

pub const VIRTIO_RNG_MSI_VECTOR: u8 = 0x50;
pub const VIRTIO_RNG_MSI_HANDLER_GPA: u64 = 0x0001_2000;
pub const VIRTIO_RNG_MSI_ADDRESS: u32 = 0xfee0_0000;
pub const VIRTIO_RNG_MSI_DATA: u16 = VIRTIO_RNG_MSI_VECTOR as u16;
pub const VIRTIO_RNG_MSI_PROOF: &[u8; 7] = b"PVNMARD";

const VIRTIO_RNG_MSI_EXIT_BUDGET: u32 = 48;
const VIRTIO_QUEUE_SIZE: u16 = 1;
const VIRTIO_QUEUE_INDEX: u16 = 0;
const NOTIFY_BARRIER: u8 = b'N';
const MSI_HANDLER_BYTE: u8 = b'M';
const ISR_ACK_BARRIER: u8 = b'A';
const COMPLETION_BARRIER: u8 = b'D';
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const VIRTIO_MSI_CAPABILITY_OFFSET: u8 = 0x74;
const VIRTIO_MSI_ADDRESS_OFFSET: u8 = 0x78;
const VIRTIO_MSI_DATA_OFFSET: u8 = 0x7c;
const PCI_CAP_ID_MSI: u32 = 0x05;
const PCI_MSI_ENABLE: u32 = 1 << 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioRngMsiCompletionGuestResult {
    io_exits: Vec<PortIoExit>,
    mmio_exits: Vec<MmioExit>,
    proof: Vec<u8>,
    completion: VirtioRngQueueCompletion,
    status: u8,
    driver_features: u64,
    queue_enabled: bool,
    payload: Vec<u8>,
    used_idx: u16,
    used_id: u32,
    used_len: u32,
    msi_address: u32,
    msi_data: u16,
    msi_delivery_count: u32,
    vector: u8,
    lapic_spiv: u32,
    completion_rflags: u64,
}

impl VirtioRngMsiCompletionGuestResult {
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
    pub const fn completion(&self) -> VirtioRngQueueCompletion {
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
    pub fn payload(&self) -> &[u8] {
        &self.payload
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
    pub const fn msi_address(&self) -> u32 {
        self.msi_address
    }

    #[must_use]
    pub const fn msi_data(&self) -> u16 {
        self.msi_data
    }

    #[must_use]
    pub const fn msi_delivery_count(&self) -> u32 {
        self.msi_delivery_count
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
    pub const fn completion_rflags(&self) -> u64 {
        self.completion_rflags
    }
}

pub fn run_virtio_rng_msi_completion_guest(
    config: VmConfig,
) -> Result<VirtioRngMsiCompletionGuestResult, Error> {
    let guest_bytes = build_guest();
    let guest = FlatGuestImage::new(
        LONG_MODE_MMIO_GUEST_ENTRY,
        LONG_MODE_MMIO_GUEST_ENTRY,
        &guest_bytes,
    )?;
    let handler_bytes = build_msi_handler();
    let handler = FlatGuestImage::new(
        GuestPhysAddr::new(VIRTIO_RNG_MSI_HANDLER_GPA),
        GuestPhysAddr::new(VIRTIO_RNG_MSI_HANDLER_GPA),
        &handler_bytes,
    )?;

    let backend = KvmBackend::open()?;
    backend.require_signal_msi_capability()?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeMmioInterruptLayout::new(
        memory.region(),
        guest.entry(),
        LONG_MODE_MMIO_STACK_POINTER,
        VIRTIO_RNG_MSI_VECTOR,
        handler.entry(),
    )
    .expect("fixed virtio-rng MSI layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(layout.interrupt_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;

    let pci = PciConfigMechanism1::with_virtio_rng_msi(VirtioRngPciFunction::new(
        VIRTIO_RNG_INTERRUPT_BAR0_GPA as u32,
    ));
    let mut port_io = PortIoBus::with_debug_port_and_pci_config(pci);
    let mut mmio = MmioBus::empty();
    mmio.register_virtio_rng_device_at(VIRTIO_RNG_INTERRUPT_BAR0_GPA)
        .expect("fixed virtio-rng MSI BAR does not overlap another MMIO device");

    let mut queue_completion = None;
    let mut delivered_message = None;
    let mut msi_delivery_count = None;
    let mut io_exits = Vec::new();
    let mut mmio_exits = Vec::new();
    let mut completed_exits = 0_u32;

    loop {
        if completed_exits >= VIRTIO_RNG_MSI_EXIT_BUDGET {
            return Err(verification_error(format!(
                "virtio-rng MSI completion exceeded exact exit budget {} before final userspace barrier",
                VIRTIO_RNG_MSI_EXIT_BUDGET
            )));
        }
        let exit = vcpu.run_once()?;
        completed_exits += 1;
        let disposition = dispatch_vcpu_exit(&mut vcpu, exit, &mut port_io, &mut mmio)?;
        match disposition {
            VmExitDisposition::Continue(continuation) => {
                if is_debug_output(&continuation, NOTIFY_BARRIER) {
                    if queue_completion.is_some() || delivered_message.is_some() {
                        return Err(verification_error(
                            "duplicate virtio-rng MSI notify completion barrier",
                        ));
                    }
                    let event = mmio.take_device_event_record().ok_or_else(|| {
                        verification_error(
                            "MSI notify barrier arrived without a pending virtio-rng queue event",
                        )
                    })?;
                    if event
                        != MmioDeviceEventRecord::new(
                            VIRTIO_RNG_INTERRUPT_BAR0_GPA,
                            MmioDeviceEvent::VirtioQueueNotified {
                                queue: VIRTIO_QUEUE_INDEX,
                            },
                        )
                    {
                        return Err(verification_error(format!(
                            "unexpected virtio-rng device event at MSI notify barrier: {event:?}"
                        )));
                    }
                    let memory = vm.guest_memory_mut().ok_or_else(|| {
                        verification_error("virtio-rng MSI VM lost registered guest memory")
                    })?;
                    let completion = mmio
                        .process_virtio_rng_notification(VIRTIO_RNG_INTERRUPT_BAR0_GPA, memory)
                        .map_err(|error| {
                            verification_error(format!(
                                "virtio-rng queue processing failed before MSI delivery: {error}"
                            ))
                        })?
                        .ok_or_else(|| {
                            verification_error("virtio-rng BAR disappeared before MSI queue processing")
                        })?;
                    let message = port_io.virtio_rng_msi_message().ok_or_else(|| {
                        verification_error(
                            "virtio-rng queue completed before guest enabled its PCI MSI message",
                        )
                    })?;
                    if message.address() != VIRTIO_RNG_MSI_ADDRESS
                        || message.data() != VIRTIO_RNG_MSI_DATA
                    {
                        return Err(verification_error(format!(
                            "guest programmed unexpected virtio-rng MSI message address={:#x} data={:#x}",
                            message.address(),
                            message.data()
                        )));
                    }
                    let delivered = vm.signal_msi(KvmMsiMessage::new(
                        u64::from(message.address()),
                        u32::from(message.data()),
                    ))?;
                    if delivered != 1 {
                        return Err(verification_error(format!(
                            "expected exactly one KVM_SIGNAL_MSI delivery, got {delivered}"
                        )));
                    }
                    queue_completion = Some(completion);
                    delivered_message = Some(message);
                    msi_delivery_count = Some(delivered);
                } else if is_debug_output(&continuation, ISR_ACK_BARRIER)
                    && (queue_completion.is_none() || delivered_message.is_none())
                {
                    return Err(verification_error(
                        "MSI ISR ACK barrier arrived before queue completion and message delivery",
                    ));
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
                    "unexpected terminal vCPU exit before virtio-rng MSI completion barrier: {report}"
                )));
            }
        }
    }

    if mmio.take_device_event_record().is_some() {
        return Err(verification_error(
            "virtio-rng MSI completion left an extra device event",
        ));
    }
    if completed_exits != VIRTIO_RNG_MSI_EXIT_BUDGET {
        return Err(verification_error(format!(
            "expected exactly {} completed exits through MSI completion barrier, got {completed_exits}",
            VIRTIO_RNG_MSI_EXIT_BUDGET
        )));
    }

    let completion = queue_completion
        .ok_or_else(|| verification_error("virtio-rng queue never completed before MSI proof"))?;
    let message = delivered_message
        .ok_or_else(|| verification_error("virtio-rng MSI message was never delivered"))?;
    let delivery_count = msi_delivery_count
        .ok_or_else(|| verification_error("virtio-rng MSI delivery count was not recorded"))?;
    validate_io_sequence(&io_exits)?;
    validate_mmio_sequence(&mmio_exits)?;

    let status = mmio
        .virtio_rng_status_at(VIRTIO_RNG_INTERRUPT_BAR0_GPA)
        .ok_or_else(|| verification_error("virtio-rng MSI status unavailable after execution"))?;
    let driver_features = mmio
        .virtio_rng_driver_features_at(VIRTIO_RNG_INTERRUPT_BAR0_GPA)
        .ok_or_else(|| verification_error("virtio-rng MSI driver features unavailable"))?;
    let queue_enabled = mmio
        .virtio_rng_queue_enabled_at(VIRTIO_RNG_INTERRUPT_BAR0_GPA)
        .ok_or_else(|| verification_error("virtio-rng MSI queue state unavailable"))?;

    let memory = vm
        .guest_memory()
        .ok_or_else(|| verification_error("virtio-rng MSI VM lost guest memory before verification"))?;
    let used_idx = read_u16(memory, VIRTIO_RNG_INTERRUPT_USED_GPA + 2)?;
    let used_id = read_u32(memory, VIRTIO_RNG_INTERRUPT_USED_GPA + 4)?;
    let used_len = read_u32(memory, VIRTIO_RNG_INTERRUPT_USED_GPA + 8)?;
    let mut payload = vec![0_u8; VIRTIO_RNG_TEST_PAYLOAD.len()];
    memory.read(
        GuestPhysAddr::new(VIRTIO_RNG_INTERRUPT_BUFFER_GPA),
        &mut payload,
    )?;

    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    let completion_rflags = vcpu.registers()?.rflags;
    let expected_status = VIRTIO_STATUS_ACKNOWLEDGE
        | VIRTIO_STATUS_DRIVER
        | VIRTIO_STATUS_FEATURES_OK
        | VIRTIO_STATUS_DRIVER_OK;

    if completion.descriptor_id() != 0
        || completion.length() != VIRTIO_RNG_TEST_PAYLOAD.len() as u32
        || status != expected_status
        || driver_features != VIRTIO_F_VERSION_1
        || !queue_enabled
        || used_idx != 1
        || used_id != 0
        || used_len != VIRTIO_RNG_TEST_PAYLOAD.len() as u32
        || payload.as_slice() != VIRTIO_RNG_TEST_PAYLOAD
        || proof.as_slice() != VIRTIO_RNG_MSI_PROOF
    {
        return Err(verification_error(format!(
            "virtio-rng MSI completion verification failed: completion={completion:?}, status={status:#x}, features={driver_features:#x}, queue_enabled={queue_enabled}, used=({used_idx},{used_id},{used_len}), payload={payload:?}, proof={proof:?}"
        )));
    }

    if completion_rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || completion_rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
    {
        return Err(verification_error(format!(
            "expected virtio-rng MSI completion barrier with architectural bit1 and IF set, got RFLAGS {completion_rflags:#x}"
        )));
    }

    Ok(VirtioRngMsiCompletionGuestResult {
        io_exits,
        mmio_exits,
        proof,
        completion,
        status,
        driver_features,
        queue_enabled,
        payload,
        used_idx,
        used_id,
        used_len,
        msi_address: message.address(),
        msi_data: message.data(),
        msi_delivery_count: delivery_count,
        vector: VIRTIO_RNG_MSI_VECTOR,
        lapic_spiv: lapic.spiv(),
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
    const READ_OFFSETS: [u8; 7] = [0x00, 0x34, 0x40, 0x50, 0x64, 0x74, 0x10];
    const WRITE_CYCLES: [(u8, u32); 3] = [
        (VIRTIO_MSI_ADDRESS_OFFSET, VIRTIO_RNG_MSI_ADDRESS),
        (VIRTIO_MSI_DATA_OFFSET, VIRTIO_RNG_MSI_DATA as u32),
        (VIRTIO_MSI_CAPABILITY_OFFSET, PCI_MSI_ENABLE | PCI_CAP_ID_MSI),
    ];
    if exits.len() != 27 {
        return Err(verification_error(format!(
            "expected 27 userspace port-I/O exits for virtio-rng MSI, got {}",
            exits.len()
        )));
    }
    for (cycle, offset) in READ_OFFSETS.into_iter().enumerate() {
        let address = &exits[cycle * 2];
        let data = &exits[cycle * 2 + 1];
        let selector = config_selector(offset);
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
                "virtio-rng MSI PCI read cycle {cycle} did not match selector {selector:#010x}"
            )));
        }
    }
    let write_base = READ_OFFSETS.len() * 2;
    for (cycle, (offset, value)) in WRITE_CYCLES.into_iter().enumerate() {
        let address = &exits[write_base + cycle * 2];
        let data = &exits[write_base + cycle * 2 + 1];
        let selector = config_selector(offset);
        if address.direction() != PortIoDirection::Out
            || address.port() != PCI_CONFIG_ADDRESS_PORT
            || address.size() != 4
            || address.count() != 1
            || address.output_data() != selector.to_le_bytes()
            || data.direction() != PortIoDirection::Out
            || data.port() != PCI_CONFIG_DATA_PORT
            || data.size() != 4
            || data.count() != 1
            || data.output_data() != value.to_le_bytes()
        {
            return Err(verification_error(format!(
                "virtio-rng MSI PCI write cycle {cycle} did not match selector {selector:#010x} value {value:#010x}"
            )));
        }
    }
    let proof_base = write_base + WRITE_CYCLES.len() * 2;
    for (exit, expected) in exits[proof_base..]
        .iter()
        .zip(VIRTIO_RNG_MSI_PROOF.iter().copied())
    {
        if exit.direction() != PortIoDirection::Out
            || exit.port() != DEBUG_PORT
            || exit.size() != 1
            || exit.count() != 1
            || exit.output_data() != [expected]
        {
            return Err(verification_error(format!(
                "virtio-rng MSI proof output did not match byte {expected:#x}"
            )));
        }
    }
    Ok(())
}

fn validate_mmio_sequence(exits: &[MmioExit]) -> Result<(), Error> {
    let expected: [(u64, MmioDirection, u32, &[u8]); 21] = [
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
            &(VIRTIO_RNG_INTERRUPT_DESCRIPTOR_GPA as u32).to_le_bytes(),
        ),
        (0x24, MmioDirection::Write, 4, &0_u32.to_le_bytes()),
        (
            0x28,
            MmioDirection::Write,
            4,
            &(VIRTIO_RNG_INTERRUPT_AVAIL_GPA as u32).to_le_bytes(),
        ),
        (0x2c, MmioDirection::Write, 4, &0_u32.to_le_bytes()),
        (
            0x30,
            MmioDirection::Write,
            4,
            &(VIRTIO_RNG_INTERRUPT_USED_GPA as u32).to_le_bytes(),
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
            "expected {} virtio-rng MSI MMIO exits, got {}",
            expected.len(),
            exits.len()
        )));
    }
    for (index, (exit, (offset, direction, length, payload))) in
        exits.iter().zip(expected).enumerate()
    {
        if exit.address() != VIRTIO_RNG_INTERRUPT_BAR0_GPA + offset
            || exit.direction() != direction
            || exit.length() != length
            || exit.write_data() != payload
        {
            return Err(verification_error(format!(
                "virtio-rng MSI MMIO exit {index} mismatch: address={:#x}, direction={:?}, length={}, data={:?}",
                exit.address(),
                exit.direction(),
                exit.length(),
                exit.write_data()
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
        operation: "virtio-rng MSI completion proof",
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

fn build_guest() -> Vec<u8> {
    let mut code = vec![0xfb, 0x90];

    emit_pci_read(&mut code, 0x00);
    emit_cmp_eax(
        &mut code,
        (u32::from(VIRTIO_RNG_PCI_DEVICE_ID) << 16) | u32::from(VIRTIO_PCI_VENDOR_ID),
    );
    emit_pci_read(&mut code, 0x34);
    emit_cmp_eax(&mut code, 0x40);
    emit_pci_read(&mut code, 0x40);
    emit_cmp_eax(&mut code, 0x0110_5009);
    emit_pci_read(&mut code, 0x50);
    emit_cmp_eax(&mut code, 0x0214_6409);
    emit_pci_read(&mut code, 0x64);
    emit_cmp_eax(&mut code, 0x0310_7409);
    emit_pci_read(&mut code, VIRTIO_MSI_CAPABILITY_OFFSET);
    emit_cmp_eax(&mut code, PCI_CAP_ID_MSI);
    emit_pci_read(&mut code, 0x10);
    code.extend_from_slice(&[0x25, 0xf0, 0xff, 0xff, 0xff]);
    emit_cmp_eax(&mut code, VIRTIO_RNG_INTERRUPT_BAR0_GPA as u32);

    emit_pci_write(&mut code, VIRTIO_MSI_ADDRESS_OFFSET, VIRTIO_RNG_MSI_ADDRESS);
    emit_pci_write(&mut code, VIRTIO_MSI_DATA_OFFSET, u32::from(VIRTIO_RNG_MSI_DATA));
    emit_pci_write(
        &mut code,
        VIRTIO_MSI_CAPABILITY_OFFSET,
        PCI_MSI_ENABLE | PCI_CAP_ID_MSI,
    );
    emit_debug(&mut code, b'P');

    emit_movabs(&mut code, 3, 0x0050_0000);
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
    emit_mmio_dword_write(&mut code, 0x20, VIRTIO_RNG_INTERRUPT_DESCRIPTOR_GPA as u32);
    emit_mmio_dword_write(&mut code, 0x24, 0);
    emit_mmio_dword_write(&mut code, 0x28, VIRTIO_RNG_INTERRUPT_AVAIL_GPA as u32);
    emit_mmio_dword_write(&mut code, 0x2c, 0);
    emit_mmio_dword_write(&mut code, 0x30, VIRTIO_RNG_INTERRUPT_USED_GPA as u32);
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
    emit_debug(&mut code, b'V');

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

fn build_msi_handler() -> Vec<u8> {
    let mut code = Vec::new();
    emit_debug(&mut code, MSI_HANDLER_BYTE);
    emit_movabs(&mut code, 3, 0x0050_0000);
    code.extend_from_slice(&[0x8a, 0x83]);
    code.extend_from_slice(&(VIRTIO_ISR_OFFSET as u32).to_le_bytes());
    emit_cmp_al(&mut code, VIRTIO_ISR_QUEUE_INTERRUPT);
    emit_debug(&mut code, ISR_ACK_BARRIER);
    code.extend_from_slice(&[0x48, 0xcf]);
    code
}

fn emit_pci_read(code: &mut Vec<u8>, offset: u8) {
    code.extend_from_slice(&[0x66, 0xba, 0xf8, 0x0c]);
    code.push(0xb8);
    code.extend_from_slice(&config_selector(offset).to_le_bytes());
    code.push(0xef);
    code.extend_from_slice(&[0x66, 0xba, 0xfc, 0x0c, 0xed]);
}

fn emit_pci_write(code: &mut Vec<u8>, offset: u8, value: u32) {
    code.extend_from_slice(&[0x66, 0xba, 0xf8, 0x0c]);
    code.push(0xb8);
    code.extend_from_slice(&config_selector(offset).to_le_bytes());
    code.push(0xef);
    code.extend_from_slice(&[0x66, 0xba, 0xfc, 0x0c]);
    code.push(0xb8);
    code.extend_from_slice(&value.to_le_bytes());
    code.push(0xef);
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
    emit_movabs(code, 7, VIRTIO_RNG_INTERRUPT_DESCRIPTOR_GPA);
    code.extend_from_slice(&[0x48, 0xc7, 0x07]);
    code.extend_from_slice(&(VIRTIO_RNG_INTERRUPT_BUFFER_GPA as u32).to_le_bytes());
    code.extend_from_slice(&[0xc7, 0x47, 0x08, 0x08, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xc7, 0x47, 0x0c, 0x02, 0x00, 0x00, 0x00]);

    emit_movabs(code, 7, VIRTIO_RNG_INTERRUPT_AVAIL_GPA);
    code.extend_from_slice(&[0xc7, 0x07, 0x00, 0x00, 0x01, 0x00]);
    code.extend_from_slice(&[0xc7, 0x47, 0x04, 0x00, 0x00, 0x00, 0x00]);

    emit_movabs(code, 7, VIRTIO_RNG_INTERRUPT_USED_GPA);
    code.extend_from_slice(&[0x48, 0xc7, 0x07, 0x00, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0xc7, 0x47, 0x08, 0x00, 0x00, 0x00, 0x00]);
}

fn emit_guest_completion_checks(code: &mut Vec<u8>) {
    emit_movabs(code, 7, VIRTIO_RNG_INTERRUPT_USED_GPA);
    code.extend_from_slice(&[0x0f, 0xb7, 0x47, 0x02, 0x83, 0xf8, 0x01]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x04, 0x85, 0xc0]);
    emit_equal_or_ud2(code);
    code.extend_from_slice(&[0x8b, 0x47, 0x08, 0x83, 0xf8, 0x08]);
    emit_equal_or_ud2(code);

    emit_movabs(code, 7, VIRTIO_RNG_INTERRUPT_BUFFER_GPA);
    code.extend_from_slice(&[0x48, 0x8b, 0x07]);
    emit_movabs(code, 1, u64::from_le_bytes(*VIRTIO_RNG_TEST_PAYLOAD));
    code.extend_from_slice(&[0x48, 0x39, 0xc8]);
    emit_equal_or_ud2(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_programs_msi_and_uses_distinct_handler_without_pic_eoi() {
        let guest = build_guest();
        let handler = build_msi_handler();
        assert!(guest.windows(4).any(|window| window == b"\xb0P\xe6\xe9"));
        assert!(guest.windows(4).any(|window| window == b"\xb0N\xe6\xe9"));
        assert!(handler.windows(4).any(|window| window == b"\xb0M\xe6\xe9"));
        assert!(handler.windows(4).any(|window| window == b"\xb0A\xe6\xe9"));
        assert!(!handler.windows(4).any(|window| window == [0xb0, 0x20, 0xe6, 0x20]));
        assert!(handler.ends_with(&[0x48, 0xcf]));
        assert!(guest.ends_with(&[0xb0, b'D', 0xe6, 0xe9, 0xf4]));
    }

    #[test]
    fn exit_budget_matches_exact_msi_userspace_contract() {
        assert_eq!(VIRTIO_RNG_MSI_EXIT_BUDGET, 48);
    }
}
