use super::pci::virtio::{
    VirtioRngPciFunction, VirtioRngQueueCompletion, VIRTIO_F_VERSION_1, VIRTIO_ISR_OFFSET,
    VIRTIO_ISR_QUEUE_INTERRUPT, VIRTIO_PCI_VENDOR_ID, VIRTIO_RNG_PCI_DEVICE_ID,
    VIRTIO_RNG_TEST_PAYLOAD, VIRTIO_STATUS_ACKNOWLEDGE, VIRTIO_STATUS_DRIVER,
    VIRTIO_STATUS_DRIVER_OK, VIRTIO_STATUS_FEATURES_OK,
};
use super::pci::{
    config_selector, PciConfigMechanism1, PCI_CONFIG_ADDRESS_PORT, PCI_CONFIG_DATA_PORT,
};
use super::{PortIoBus, DEBUG_PORT};
use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError};
use crate::execution::run_vcpu_until_stopped_with_mmio_observer;
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
use crate::vcpu::{MmioDirection, MmioExit, PortIoDirection, PortIoExit, VcpuExit, VcpuId};
use crate::vmexit::{VmExitContinuation, VmExitReport};
use std::io;

pub const VIRTIO_RNG_INTERRUPT_BAR0_GPA: u64 = LONG_MODE_MMIO_DEVICE_GPA;
pub const VIRTIO_RNG_INTERRUPT_DESCRIPTOR_GPA: u64 = 0x0001_8000;
pub const VIRTIO_RNG_INTERRUPT_AVAIL_GPA: u64 = 0x0001_8100;
pub const VIRTIO_RNG_INTERRUPT_USED_GPA: u64 = 0x0001_8200;
pub const VIRTIO_RNG_INTERRUPT_BUFFER_GPA: u64 = 0x0001_8300;
pub const VIRTIO_RNG_INTERRUPT_PROOF: &[u8; 6] = b"PVNIAR";

const VIRTIO_RNG_INTERRUPT_EXIT_BUDGET: u32 = 40;
const VIRTIO_QUEUE_SIZE: u16 = 1;
const VIRTIO_QUEUE_INDEX: u16 = 0;
const NOTIFY_BARRIER: u8 = b'N';
const ISR_HANDLER_BYTE: u8 = b'I';
const ISR_ACK_BARRIER: u8 = b'A';
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtioRngCompletionInterruptGuestResult {
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
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    assert_count: u32,
    deassert_count: u32,
    terminal_rip: u64,
    report: VmExitReport,
}

impl VirtioRngCompletionInterruptGuestResult {
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
    pub const fn terminal_rip(&self) -> u64 {
        self.terminal_rip
    }

    #[must_use]
    pub const fn report(&self) -> VmExitReport {
        self.report
    }
}

pub fn run_virtio_rng_completion_interrupt_guest(
    config: VmConfig,
) -> Result<VirtioRngCompletionInterruptGuestResult, Error> {
    let guest_bytes = build_guest();
    let terminal_rip = LONG_MODE_MMIO_GUEST_ENTRY.get()
        + u64::try_from(guest_bytes.len()).expect("fixed guest length fits u64");
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
    .expect("fixed virtio-rng interrupt layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(layout.interrupt_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;

    let pci = PciConfigMechanism1::with_virtio_rng(VirtioRngPciFunction::new(
        VIRTIO_RNG_INTERRUPT_BAR0_GPA as u32,
    ));
    let mut port_io = PortIoBus::with_debug_port_and_pci_config(pci);
    let mut mmio = MmioBus::empty();
    mmio.register_virtio_rng_device_at(VIRTIO_RNG_INTERRUPT_BAR0_GPA)
        .expect("fixed virtio-rng BAR does not overlap another MMIO device");

    let mut queue_completion = None;
    let mut line_asserted = false;
    let mut assert_count = 0_u32;
    let mut deassert_count = 0_u32;

    let execution = run_vcpu_until_stopped_with_mmio_observer(
        &mut vcpu,
        &mut port_io,
        &mut mmio,
        VIRTIO_RNG_INTERRUPT_EXIT_BUDGET,
        |continuation, mmio| {
            if is_debug_output(continuation, NOTIFY_BARRIER) {
                if queue_completion.is_some() || line_asserted {
                    return Err(verification_error(
                        "duplicate virtio-rng completion notify barrier",
                    ));
                }
                let event = mmio.take_device_event_record().ok_or_else(|| {
                    verification_error(
                        "notify barrier arrived without a pending virtio-rng queue event",
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
                        "unexpected virtio-rng device event at notify barrier: {event:?}"
                    )));
                }
                let memory = vm.guest_memory_mut().ok_or_else(|| {
                    verification_error("virtio-rng VM lost registered guest memory")
                })?;
                let completion = mmio
                    .process_virtio_rng_notification(VIRTIO_RNG_INTERRUPT_BAR0_GPA, memory)
                    .map_err(|error| {
                        verification_error(format!(
                            "virtio-rng queue processing failed before interrupt: {error}"
                        ))
                    })?
                    .ok_or_else(|| {
                        verification_error("virtio-rng BAR disappeared before queue processing")
                    })?;
                vm.set_gsi_level(KvmBackend::IRQCHIP_GSI, true)?;
                queue_completion = Some(completion);
                line_asserted = true;
                assert_count += 1;
            } else if is_debug_output(continuation, ISR_ACK_BARRIER) {
                if queue_completion.is_none() || !line_asserted {
                    return Err(verification_error(
                        "virtio-rng ISR ACK barrier arrived without an asserted completion line",
                    ));
                }
                vm.set_gsi_level(KvmBackend::IRQCHIP_GSI, false)?;
                line_asserted = false;
                deassert_count += 1;
            }
            Ok(())
        },
    )?;

    if line_asserted {
        return Err(verification_error(
            "virtio-rng completion left INTx GSI asserted after ISR acknowledgement",
        ));
    }
    if assert_count != 1 || deassert_count != 1 {
        return Err(verification_error(format!(
            "expected one INTx assert/deassert lifecycle, got assert={assert_count}, deassert={deassert_count}"
        )));
    }
    if mmio.take_device_event_record().is_some() {
        return Err(verification_error(
            "virtio-rng completion execution left an extra device event",
        ));
    }

    let completion = queue_completion
        .ok_or_else(|| verification_error("virtio-rng queue was never processed after notify"))?;
    validate_io_sequence(execution.io_exits())?;
    validate_mmio_sequence(execution.mmio_exits())?;

    let status = mmio
        .virtio_rng_status_at(VIRTIO_RNG_INTERRUPT_BAR0_GPA)
        .ok_or_else(|| verification_error("virtio-rng status unavailable after execution"))?;
    let driver_features = mmio
        .virtio_rng_driver_features_at(VIRTIO_RNG_INTERRUPT_BAR0_GPA)
        .ok_or_else(|| {
            verification_error("virtio-rng driver features unavailable after execution")
        })?;
    let queue_enabled = mmio
        .virtio_rng_queue_enabled_at(VIRTIO_RNG_INTERRUPT_BAR0_GPA)
        .ok_or_else(|| verification_error("virtio-rng queue state unavailable after execution"))?;

    let memory = vm
        .guest_memory()
        .ok_or_else(|| verification_error("virtio-rng VM lost guest memory before verification"))?;
    let used_idx = read_u16(memory, VIRTIO_RNG_INTERRUPT_USED_GPA + 2)?;
    let used_id = read_u32(memory, VIRTIO_RNG_INTERRUPT_USED_GPA + 4)?;
    let used_len = read_u32(memory, VIRTIO_RNG_INTERRUPT_USED_GPA + 8)?;
    let mut payload = vec![0_u8; VIRTIO_RNG_TEST_PAYLOAD.len()];
    memory.read(
        GuestPhysAddr::new(VIRTIO_RNG_INTERRUPT_BUFFER_GPA),
        &mut payload,
    )?;

    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    let report = execution.report();
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
        || proof.as_slice() != VIRTIO_RNG_INTERRUPT_PROOF
    {
        return Err(verification_error(format!(
            "virtio-rng completion interrupt verification failed: completion={completion:?}, status={status:#x}, features={driver_features:#x}, queue_enabled={queue_enabled}, used=({used_idx},{used_id},{used_len}), payload={payload:?}, proof={proof:?}"
        )));
    }

    if report.exit() != VcpuExit::Hlt
        || report.rip() != terminal_rip
        || report.rflags() & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || report.rflags() & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
    {
        return Err(verification_error(format!(
            "expected virtio-rng completion HLT at RIP {terminal_rip:#x} with architectural bit1 and IF set, got {report}"
        )));
    }

    Ok(VirtioRngCompletionInterruptGuestResult {
        io_exits: execution.io_exits().to_vec(),
        mmio_exits: execution.mmio_exits().to_vec(),
        proof,
        completion,
        status,
        driver_features,
        queue_enabled,
        payload,
        used_idx,
        used_id,
        used_len,
        gsi: KvmBackend::IRQCHIP_GSI,
        vector: KvmBackend::IRQCHIP_VECTOR,
        lapic_spiv: lapic.spiv(),
        lapic_lint0: lapic.lint0(),
        assert_count,
        deassert_count,
        terminal_rip,
        report,
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
    let selectors = [0x00, 0x34, 0x40, 0x50, 0x64, 0x10].map(config_selector);
    if exits.len() != 18 {
        return Err(verification_error(format!(
            "expected 18 userspace port-I/O exits, got {}",
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
                "virtio completion PCI config cycle {cycle} did not match selector {selector:#010x}"
            )));
        }
    }
    for (exit, expected) in exits[12..]
        .iter()
        .zip(VIRTIO_RNG_INTERRUPT_PROOF.iter().copied())
    {
        if exit.direction() != PortIoDirection::Out
            || exit.port() != DEBUG_PORT
            || exit.size() != 1
            || exit.count() != 1
            || exit.output_data() != [expected]
        {
            return Err(verification_error(format!(
                "virtio completion proof output did not match byte {expected:#x}"
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
            "expected {} virtio completion MMIO exits, got {}",
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
                "virtio completion MMIO exit {index} mismatch: address={:#x}, direction={:?}, length={}, data={:?}",
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
        operation: "virtio-rng completion interrupt proof",
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
        (u32::from(VIRTIO_RNG_PCI_DEVICE_ID) << 16) | u32::from(VIRTIO_PCI_VENDOR_ID),
    );
    emit_pci_read(&mut code, 0x34);
    emit_cmp_eax(&mut code, 0x40);
    emit_pci_read(&mut code, 0x40);
    emit_cmp_eax(&mut code, 0x0110_5009);
    emit_pci_read(&mut code, 0x50);
    emit_cmp_eax(&mut code, 0x0214_6409);
    emit_pci_read(&mut code, 0x64);
    emit_cmp_eax(&mut code, 0x0310_0009);
    emit_pci_read(&mut code, 0x10);
    code.extend_from_slice(&[0x25, 0xf0, 0xff, 0xff, 0xff]);
    emit_cmp_eax(&mut code, VIRTIO_RNG_INTERRUPT_BAR0_GPA as u32);
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
    fn fixture_contains_pic_setup_isr_reads_and_terminal_hlt() {
        let guest = build_guest();
        let handler = build_interrupt_handler();
        assert_eq!(guest.last(), Some(&0xf4));
        assert!(guest.windows(4).any(|window| window == b"\xb0P\xe6\xe9"));
        assert!(guest.windows(4).any(|window| window == b"\xb0V\xe6\xe9"));
        assert!(guest.windows(4).any(|window| window == b"\xb0N\xe6\xe9"));
        assert!(guest.windows(4).any(|window| window == b"\xb0R\xe6\xe9"));
        assert!(handler.windows(4).any(|window| window == b"\xb0I\xe6\xe9"));
        assert!(handler.windows(4).any(|window| window == b"\xb0A\xe6\xe9"));
        assert!(handler.ends_with(&[0x48, 0xcf]));
    }

    #[test]
    fn exit_budget_matches_exact_userspace_exit_contract() {
        assert_eq!(VIRTIO_RNG_INTERRUPT_EXIT_BUDGET, 40);
    }
}
