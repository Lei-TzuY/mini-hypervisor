use super::interrupt::LongModeMmioInterruptLayout;
use super::long_mode::{LONG_MODE_MMIO_DEVICE_GPA, LONG_MODE_MMIO_STACK_POINTER};
use super::{
    MmioBus, MmioDeviceEvent, MmioService, LEVEL_INTERRUPT_ACK_OFFSET,
    LEVEL_INTERRUPT_STATUS_OFFSET, LEVEL_INTERRUPT_STATUS_PENDING,
};
use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError, VmExitError};
use crate::interrupt::{
    LONG_MODE_INTERRUPT_GUEST_ENTRY, LONG_MODE_INTERRUPT_HANDLER, LONG_MODE_INTERRUPT_VECTOR,
    X86_RFLAGS_INTERRUPT_ENABLE,
};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::vcpu::{MmioDirection, MmioExit, PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId};
use std::io;

pub const MMIO_LEVEL_INTERRUPT_COMMAND_VALUE: u8 = b'W';
pub const MMIO_LEVEL_INTERRUPT_ACK_VALUE: u8 = 1;
pub const MMIO_LEVEL_INTERRUPT_PROOF: &[u8; 6] = b"AISCMD";

const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const MMIO_LEVEL_INTERRUPT_ARMED_BYTE: u8 = b'A';
const MMIO_LEVEL_INTERRUPT_HANDLER_BYTE: u8 = b'I';
const MMIO_LEVEL_INTERRUPT_STATUS_BYTE: u8 = b'S';
const MMIO_LEVEL_INTERRUPT_ACK_COMMITTED_BYTE: u8 = b'C';
const MMIO_LEVEL_INTERRUPT_RESUMED_BYTE: u8 = b'M';
const MMIO_LEVEL_INTERRUPT_DONE_BYTE: u8 = b'D';
const MMIO_LEVEL_INTERRUPT_FAILURE_BYTE: u8 = b'F';

// The main path initializes the legacy PIC exactly as the established irqchip fixtures do, enables
// interrupts, writes one command through virtual MMIO, and exposes explicit userspace commit
// barriers before and after the interrupt lifecycle. HLT is only a safety fallback; userspace stops
// at D because in-kernel LAPIC HLT is not a portable userspace-exit contract.
const MMIO_LEVEL_INTERRUPT_GUEST_BYTES: [u8; 65] = [
    0xfa, // cli
    0xb0, 0x11, 0xe6, 0x20, 0xe6, 0xa0, // ICW1: initialize master and slave PICs
    0xb0, 0x40, 0xe6, 0x21, // ICW2: master IRQ0..7 -> vectors 0x40..0x47
    0xb0, 0x48, 0xe6, 0xa1, // ICW2: slave IRQ8..15 -> vectors 0x48..0x4f
    0xb0, 0x04, 0xe6, 0x21, // ICW3: master has slave on IRQ2
    0xb0, 0x02, 0xe6, 0xa1, // ICW3: slave cascade identity 2
    0xb0, 0x01, 0xe6, 0x21, 0xe6, 0xa1, // ICW4: 8086 mode on both PICs
    0xb0, 0xfe, 0xe6, 0x21, // OCW1: unmask only master IRQ0
    0xb0, 0xff, 0xe6, 0xa1, // OCW1: mask every slave IRQ
    0xfb, // sti
    0x90, // nop -- complete STI's one-instruction interrupt shadow
    0x48, 0xbb, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs $0x500000, %rbx
    0xc6, 0x03, MMIO_LEVEL_INTERRUPT_COMMAND_VALUE, // command register write
    0xb0, MMIO_LEVEL_INTERRUPT_ARMED_BYTE, 0xe6, 0xe9, // command completion barrier
    0xb0, MMIO_LEVEL_INTERRUPT_RESUMED_BYTE, 0xe6, 0xe9, // resumed main after IRETQ
    0xb0, MMIO_LEVEL_INTERRUPT_DONE_BYTE, 0xe6, 0xe9, // completion barrier
    0xf4, // safety fallback
];

// RBX retains the virtual device base across interrupt entry. The handler proves the device is
// still pending by reading STATUS=1, ACKs it through MMIO, exposes an ACK-completion barrier, then
// sends PIC EOI and returns. If STATUS is not 1, F is emitted instead of S and userspace fails the
// exact proof before re-entering the fallback HLT.
const MMIO_LEVEL_INTERRUPT_HANDLER_BYTES: [u8; 34] = [
    0xb0, MMIO_LEVEL_INTERRUPT_HANDLER_BYTE, 0xe6, 0xe9, // I
    0x8a, 0x43, 0x01, // mov 1(%rbx), %al -- STATUS read
    0x3c, LEVEL_INTERRUPT_STATUS_PENDING, // cmp $1, %al
    0x75, 0x12, // jne failure output at offset 29
    0xb0, MMIO_LEVEL_INTERRUPT_STATUS_BYTE, 0xe6, 0xe9, // S
    0xc6, 0x43, 0x02, MMIO_LEVEL_INTERRUPT_ACK_VALUE, // ACK write
    0xb0, MMIO_LEVEL_INTERRUPT_ACK_COMMITTED_BYTE, 0xe6, 0xe9, // C
    0xb0, 0x20, 0xe6, 0x20, // non-specific EOI to master PIC
    0x48, 0xcf, // iretq
    0xb0, MMIO_LEVEL_INTERRUPT_FAILURE_BYTE, 0xe6, 0xe9, // F
    0xf4, // safety fallback; exact proof fails on F before another re-entry
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongModeMmioLevelInterruptGuestResult {
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    armed_rflags: u64,
    completion_rflags: u64,
    assert_event_count: u32,
    deassert_event_count: u32,
    command_exit: MmioExit,
    status_exit: MmioExit,
    ack_exit: MmioExit,
    io_exits: Vec<PortIoExit>,
    writes: Vec<u8>,
    proof: Vec<u8>,
}

impl LongModeMmioLevelInterruptGuestResult {
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
    pub const fn armed_rflags(&self) -> u64 {
        self.armed_rflags
    }

    #[must_use]
    pub const fn completion_rflags(&self) -> u64 {
        self.completion_rflags
    }

    #[must_use]
    pub const fn assert_event_count(&self) -> u32 {
        self.assert_event_count
    }

    #[must_use]
    pub const fn deassert_event_count(&self) -> u32 {
        self.deassert_event_count
    }

    #[must_use]
    pub const fn command_exit(&self) -> &MmioExit {
        &self.command_exit
    }

    #[must_use]
    pub const fn status_exit(&self) -> &MmioExit {
        &self.status_exit
    }

    #[must_use]
    pub const fn ack_exit(&self) -> &MmioExit {
        &self.ack_exit
    }

    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn writes(&self) -> &[u8] {
        &self.writes
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
}

pub fn run_long_mode_mmio_level_interrupt_guest(
    config: VmConfig,
) -> Result<LongModeMmioLevelInterruptGuestResult, Error> {
    let guest = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_GUEST_ENTRY,
        LONG_MODE_INTERRUPT_GUEST_ENTRY,
        &MMIO_LEVEL_INTERRUPT_GUEST_BYTES,
    )?;
    let handler = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_HANDLER,
        LONG_MODE_INTERRUPT_HANDLER,
        &MMIO_LEVEL_INTERRUPT_HANDLER_BYTES,
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
    .expect("fixed MMIO level-interrupt fixture layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(layout.interrupt_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;
    let mut port_io = PortIoBus::with_debug_port();
    let mut mmio = MmioBus::with_level_interrupt_byte_device_at(LONG_MODE_MMIO_DEVICE_GPA);

    let command_exit = expect_mmio(&mut vcpu, "MMIO level interrupt command")?;
    validate_write(
        &command_exit,
        LONG_MODE_MMIO_DEVICE_GPA,
        MMIO_LEVEL_INTERRUPT_COMMAND_VALUE,
        "MMIO level interrupt command",
    )?;
    require_write_service(
        mmio.dispatch(&command_exit)?,
        "MMIO level interrupt command service",
    )?;

    // A later KVM_RUN is required to complete the pending command MMIO. Only the explicit A output
    // makes the device's assert request eligible to drive the host GSI line.
    let armed_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        MMIO_LEVEL_INTERRUPT_ARMED_BYTE,
        "MMIO level interrupt armed barrier",
    )?;
    let armed = vcpu.registers()?;
    require_interrupt_enabled_flags("MMIO level interrupt armed state", armed.rflags)?;

    require_event(
        mmio.take_device_event(),
        MmioDeviceEvent::InterruptLineAssertRequested,
        "MMIO level interrupt assert event",
    )?;
    require_no_event(
        mmio.take_device_event(),
        "MMIO level interrupt duplicate assert event",
    )?;
    vm.set_gsi_level(KvmBackend::IRQCHIP_GSI, true)?;

    let handler_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        MMIO_LEVEL_INTERRUPT_HANDLER_BYTE,
        "MMIO level interrupt handler entry",
    )?;

    let status_exit = expect_mmio(&mut vcpu, "MMIO level interrupt status read")?;
    validate_read(
        &status_exit,
        LONG_MODE_MMIO_DEVICE_GPA + LEVEL_INTERRUPT_STATUS_OFFSET,
        "MMIO level interrupt status read",
    )?;
    let status_response = match mmio.dispatch(&status_exit)? {
        MmioService::Read(response) if response == [LEVEL_INTERRUPT_STATUS_PENDING] => response,
        MmioService::Read(response) => {
            return Err(verification_error(
                "MMIO level interrupt status service",
                format!(
                    "expected pending status byte {}, got {:?}",
                    LEVEL_INTERRUPT_STATUS_PENDING, response
                ),
            ));
        }
        MmioService::Write => {
            return Err(verification_error(
                "MMIO level interrupt status service",
                "status read unexpectedly resolved as a write service",
            ));
        }
    };
    vcpu.write_mmio_read_response(&status_response)?;

    // S can only execute after KVM consumes the supplied STATUS response. The guest compares AL
    // against 1 and emits F on mismatch, so requiring S is guest-side evidence that STATUS=1 was
    // actually observed, not merely prepared in userspace.
    let status_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        MMIO_LEVEL_INTERRUPT_STATUS_BYTE,
        "MMIO level interrupt status completion",
    )?;

    let ack_exit = expect_mmio(&mut vcpu, "MMIO level interrupt ACK write")?;
    validate_write(
        &ack_exit,
        LONG_MODE_MMIO_DEVICE_GPA + LEVEL_INTERRUPT_ACK_OFFSET,
        MMIO_LEVEL_INTERRUPT_ACK_VALUE,
        "MMIO level interrupt ACK write",
    )?;
    require_write_service(
        mmio.dispatch(&ack_exit)?,
        "MMIO level interrupt ACK service",
    )?;

    // ACK is still in-flight at KVM_EXIT_MMIO. C is the mandatory later-run barrier; deasserting
    // the line before C would repeat the exact pending-MMIO correctness bug rejected in #82.
    let ack_committed_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        MMIO_LEVEL_INTERRUPT_ACK_COMMITTED_BYTE,
        "MMIO level interrupt ACK completion barrier",
    )?;
    require_event(
        mmio.take_device_event(),
        MmioDeviceEvent::InterruptLineDeassertRequested,
        "MMIO level interrupt deassert event",
    )?;
    require_no_event(
        mmio.take_device_event(),
        "MMIO level interrupt duplicate deassert event",
    )?;
    vm.set_gsi_level(KvmBackend::IRQCHIP_GSI, false)?;

    // After the line is low, the next re-entry lets the handler issue PIC EOI and IRETQ. M proves
    // return to the interrupted main path; D is the following userspace barrier that commits M.
    let resumed_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        MMIO_LEVEL_INTERRUPT_RESUMED_BYTE,
        "MMIO level interrupt resumed main",
    )?;
    let completion_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        MMIO_LEVEL_INTERRUPT_DONE_BYTE,
        "MMIO level interrupt completion barrier",
    )?;
    let completion = vcpu.registers()?;
    require_interrupt_enabled_flags("MMIO level interrupt completion state", completion.rflags)?;

    let writes = mmio.writes().unwrap_or(&[]).to_vec();
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    let io_exits = vec![
        armed_io,
        handler_io,
        status_io,
        ack_committed_io,
        resumed_io,
        completion_io,
    ];
    if writes.as_slice() != [MMIO_LEVEL_INTERRUPT_COMMAND_VALUE, MMIO_LEVEL_INTERRUPT_ACK_VALUE]
        || proof.as_slice() != MMIO_LEVEL_INTERRUPT_PROOF
        || io_exits.len() != MMIO_LEVEL_INTERRUPT_PROOF.len()
    {
        return Err(verification_error(
            "MMIO level interrupt execution proof",
            format!(
                "expected writes {:?} and proof {:?}; got writes {:?} and proof {:?}",
                [MMIO_LEVEL_INTERRUPT_COMMAND_VALUE, MMIO_LEVEL_INTERRUPT_ACK_VALUE],
                MMIO_LEVEL_INTERRUPT_PROOF,
                writes,
                proof
            ),
        ));
    }

    Ok(LongModeMmioLevelInterruptGuestResult {
        gsi: KvmBackend::IRQCHIP_GSI,
        vector: KvmBackend::IRQCHIP_VECTOR,
        lapic_spiv: lapic.spiv(),
        lapic_lint0: lapic.lint0(),
        armed_rflags: armed.rflags,
        completion_rflags: completion.rflags,
        assert_event_count: 1,
        deassert_event_count: 1,
        command_exit,
        status_exit,
        ack_exit,
        io_exits,
        writes,
        proof,
    })
}

fn expect_mmio(vcpu: &mut Vcpu, stage: &'static str) -> Result<MmioExit, Error> {
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Mmio {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage,
            expected_reason: VcpuExit::Mmio.reason(),
            actual_reason: exit.reason(),
        }));
    }
    vcpu.mmio_exit()
}

fn validate_write(
    exit: &MmioExit,
    address: u64,
    expected: u8,
    stage: &'static str,
) -> Result<(), Error> {
    if exit.direction() != MmioDirection::Write
        || exit.address() != address
        || exit.length() != 1
        || exit.write_data() != [expected]
    {
        return Err(verification_error(
            stage,
            format!(
                "expected byte write {:?} to {address:#x}, got direction {:?}, address {:#x}, length {}, data {:?}",
                [expected],
                exit.direction(),
                exit.address(),
                exit.length(),
                exit.write_data()
            ),
        ));
    }
    Ok(())
}

fn validate_read(exit: &MmioExit, address: u64, stage: &'static str) -> Result<(), Error> {
    if exit.direction() != MmioDirection::Read || exit.address() != address || exit.length() != 1 {
        return Err(verification_error(
            stage,
            format!(
                "expected byte read from {address:#x}, got direction {:?}, address {:#x}, length {}",
                exit.direction(),
                exit.address(),
                exit.length()
            ),
        ));
    }
    Ok(())
}

fn require_write_service(service: MmioService, stage: &'static str) -> Result<(), Error> {
    if service != MmioService::Write {
        return Err(verification_error(
            stage,
            "MMIO write unexpectedly produced a read response",
        ));
    }
    Ok(())
}

fn require_event(
    event: Option<MmioDeviceEvent>,
    expected: MmioDeviceEvent,
    stage: &'static str,
) -> Result<(), Error> {
    if event != Some(expected) {
        return Err(verification_error(
            stage,
            format!("expected device event {expected:?}, got {event:?}"),
        ));
    }
    Ok(())
}

fn require_no_event(event: Option<MmioDeviceEvent>, stage: &'static str) -> Result<(), Error> {
    if let Some(event) = event {
        return Err(verification_error(
            stage,
            format!("expected no additional device event, got {event:?}"),
        ));
    }
    Ok(())
}

fn run_expected_debug_output(
    vcpu: &mut Vcpu,
    port_io: &mut PortIoBus,
    expected: u8,
    stage: &'static str,
) -> Result<PortIoExit, Error> {
    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Io {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage,
            expected_reason: VcpuExit::Io.reason(),
            actual_reason: exit.reason(),
        }));
    }
    let io_exit = vcpu.port_io_exit()?;
    if io_exit.direction() != PortIoDirection::Out
        || io_exit.size() != 1
        || io_exit.port() != DEBUG_PORT
        || io_exit.count() != 1
        || io_exit.output_data() != [expected]
    {
        return Err(verification_error(
            stage,
            format!(
                "expected byte-wide debug output {:?}, got direction {:?}, size {}, port {:#x}, count {}, data {:?}",
                char::from(expected),
                io_exit.direction(),
                io_exit.size(),
                io_exit.port(),
                io_exit.count(),
                io_exit.output_data()
            ),
        ));
    }
    if port_io.dispatch(&io_exit)? != PortIoService::Output {
        return Err(verification_error(
            stage,
            "debug output unexpectedly requested an input response",
        ));
    }
    Ok(io_exit)
}

fn require_interrupt_enabled_flags(operation: &'static str, rflags: u64) -> Result<(), Error> {
    if rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
        || rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
    {
        return Err(verification_error(
            operation,
            format!("expected architectural RFLAGS bit 1 and IF set, got RFLAGS {rflags:#x}"),
        ));
    }
    Ok(())
}

fn verification_error(operation: &'static str, detail: impl Into<String>) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: VcpuId::BOOT.get(),
        operation,
        source: io::Error::new(io::ErrorKind::InvalidData, detail.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_interrupt_guest_and_handler_contract_are_stable() {
        assert_eq!(MMIO_LEVEL_INTERRUPT_GUEST_BYTES.len(), 65);
        assert_eq!(
            &MMIO_LEVEL_INTERRUPT_GUEST_BYTES[49..52],
            &[0xc6, 0x03, MMIO_LEVEL_INTERRUPT_COMMAND_VALUE]
        );
        assert_eq!(
            &MMIO_LEVEL_INTERRUPT_GUEST_BYTES[52..56],
            &[0xb0, b'A', 0xe6, 0xe9]
        );
        assert_eq!(MMIO_LEVEL_INTERRUPT_HANDLER_BYTES.len(), 34);
        assert_eq!(
            &MMIO_LEVEL_INTERRUPT_HANDLER_BYTES[4..9],
            &[0x8a, 0x43, 0x01, 0x3c, LEVEL_INTERRUPT_STATUS_PENDING]
        );
        assert_eq!(&MMIO_LEVEL_INTERRUPT_HANDLER_BYTES[9..11], &[0x75, 0x12]);
        assert_eq!(
            &MMIO_LEVEL_INTERRUPT_HANDLER_BYTES[15..19],
            &[0xc6, 0x43, 0x02, MMIO_LEVEL_INTERRUPT_ACK_VALUE]
        );
        assert_eq!(MMIO_LEVEL_INTERRUPT_PROOF, b"AISCMD");
    }
}
