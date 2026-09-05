use super::long_mode::{
    LongModeMmioBootLayout, LongModeMmioConfigurationError, LONG_MODE_MMIO_DEVICE_GPA,
    LONG_MODE_MMIO_STACK_POINTER, LONG_MODE_MMIO_VIRTUAL_PAGE,
};
use super::{MmioBus, MmioDeviceEvent, MmioService};
use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError, VmExitError};
use crate::interrupt::{
    LongModeInterruptConfigurationError, LongModeInterruptLayout, LONG_MODE_INTERRUPT_GUEST_ENTRY,
    LONG_MODE_INTERRUPT_HANDLER, LONG_MODE_INTERRUPT_IDT_ADDR, LONG_MODE_INTERRUPT_VECTOR,
    X86_RFLAGS_INTERRUPT_ENABLE,
};
use crate::kvm::KvmBackend;
use crate::loader::FlatGuestImage;
use crate::long_mode::{
    LONG_MODE_ALIAS_PT_ADDR, LONG_MODE_ALIAS_VIRTUAL_BASE, LONG_MODE_IDENTITY_MAP_SIZE,
    LONG_MODE_PAGE_SIZE,
};
use crate::memory::{GuestMemory, GuestMemoryRegion, GuestPhysAddr};
use crate::portio::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::vcpu::{
    MmioDirection, MmioExit, PortIoDirection, PortIoExit, Vcpu, VcpuExit, VcpuId,
};
use std::fmt;
use std::io;

pub const MMIO_INTERRUPT_WRITE_VALUE: u8 = b'W';
pub const MMIO_INTERRUPT_PROOF: &[u8; 3] = b"IMD";

const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const MMIO_INTERRUPT_HANDLER_BYTE: u8 = b'I';
const MMIO_INTERRUPT_RESUMED_BYTE: u8 = b'M';
const MMIO_INTERRUPT_DONE_BYTE: u8 = b'D';
const INTERRUPT_GATE_SIZE: u64 = 16;

const MMIO_INTERRUPT_GUEST_BYTES: [u8; 61] = [
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
    0xc6, 0x03, MMIO_INTERRUPT_WRITE_VALUE, // movb $'W', (%rbx) -> KVM_EXIT_MMIO
    0xb0, MMIO_INTERRUPT_RESUMED_BYTE, 0xe6, 0xe9, // resumed main after IRQ + IRETQ
    0xb0, MMIO_INTERRUPT_DONE_BYTE, 0xe6, 0xe9, // completion barrier commits M
    0xf4, // safety fallback; host deliberately stops at D
];

const MMIO_INTERRUPT_HANDLER_BYTES: [u8; 10] = [
    0xb0, MMIO_INTERRUPT_HANDLER_BYTE, 0xe6, 0xe9, // interrupt handler proof
    0xb0, 0x20, 0xe6, 0x20, // non-specific EOI to master PIC
    0x48, 0xcf, // iretq
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LongModeMmioInterruptConfigurationError {
    Mmio(LongModeMmioConfigurationError),
    Interrupt(LongModeInterruptConfigurationError),
}

impl fmt::Display for LongModeMmioInterruptConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mmio(error) => error.fmt(f),
            Self::Interrupt(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for LongModeMmioInterruptConfigurationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mmio(error) => Some(error),
            Self::Interrupt(error) => Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongModeMmioInterruptLayout {
    mmio: LongModeMmioBootLayout,
    interrupt: LongModeInterruptLayout,
}

impl LongModeMmioInterruptLayout {
    pub fn new(
        memory: GuestMemoryRegion,
        entry: GuestPhysAddr,
        stack_pointer: u64,
        vector: u8,
        handler: GuestPhysAddr,
    ) -> Result<Self, LongModeMmioInterruptConfigurationError> {
        let mmio = LongModeMmioBootLayout::new(memory, entry, stack_pointer)
            .map_err(LongModeMmioInterruptConfigurationError::Mmio)?;
        let interrupt = LongModeInterruptLayout::new(
            memory,
            entry,
            stack_pointer,
            vector,
            handler,
        )
        .map_err(LongModeMmioInterruptConfigurationError::Interrupt)?;
        Ok(Self { mmio, interrupt })
    }

    #[must_use]
    pub const fn mmio_layout(&self) -> &LongModeMmioBootLayout {
        &self.mmio
    }

    #[must_use]
    pub const fn interrupt_layout(&self) -> &LongModeInterruptLayout {
        &self.interrupt
    }

    pub(crate) fn install_tables(&self, memory: &mut GuestMemory) -> Result<(), Error> {
        // Install descriptor tables first, then re-materialize the same bootstrap page tables with
        // the virtual-MMIO PTE. The second operation touches only 0x1000..0x5000, so the fixed GDT
        // and IDT at 0x5000/0x6000 remain intact while the final page-table state gains the device
        // mapping.
        self.interrupt.install_tables(memory)?;
        self.mmio.install_page_tables(memory)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongModeMmioInterruptGuestResult {
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    event_rflags: u64,
    completion_rflags: u64,
    device_event_count: u32,
    mmio_exit: MmioExit,
    io_exits: Vec<PortIoExit>,
    writes: Vec<u8>,
    proof: Vec<u8>,
}

impl LongModeMmioInterruptGuestResult {
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
    pub const fn event_rflags(&self) -> u64 {
        self.event_rflags
    }

    #[must_use]
    pub const fn completion_rflags(&self) -> u64 {
        self.completion_rflags
    }

    #[must_use]
    pub const fn device_event_count(&self) -> u32 {
        self.device_event_count
    }

    #[must_use]
    pub const fn mmio_exit(&self) -> &MmioExit {
        &self.mmio_exit
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

pub fn run_long_mode_mmio_interrupt_guest(
    config: VmConfig,
) -> Result<LongModeMmioInterruptGuestResult, Error> {
    let guest = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_GUEST_ENTRY,
        LONG_MODE_INTERRUPT_GUEST_ENTRY,
        &MMIO_INTERRUPT_GUEST_BYTES,
    )?;
    let handler = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_HANDLER,
        LONG_MODE_INTERRUPT_HANDLER,
        &MMIO_INTERRUPT_HANDLER_BYTES,
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
    .expect("fixed MMIO-device interrupt fixture layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(layout.interrupt_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;
    let mut port_io = PortIoBus::with_debug_port();
    let mut mmio = MmioBus::with_interrupting_byte_device_at(LONG_MODE_MMIO_DEVICE_GPA, b'R');

    let exit = vcpu.run_once()?;
    if exit != VcpuExit::Mmio {
        return Err(Error::VmExit(VmExitError::UnexpectedSequence {
            stage: "MMIO device interrupt trigger",
            expected_reason: VcpuExit::Mmio.reason(),
            actual_reason: exit.reason(),
        }));
    }
    let mmio_exit = vcpu.mmio_exit()?;
    validate_trigger_mmio(&mmio_exit)?;
    if mmio.dispatch(&mmio_exit)? != MmioService::Write {
        return Err(verification_error(
            "MMIO device interrupt trigger",
            "interrupting device write unexpectedly returned a read response",
        ));
    }

    let event_registers = vcpu.registers()?;
    require_interrupt_enabled_flags("MMIO device interrupt event state", event_registers.rflags)?;
    if mmio.take_device_event() != Some(MmioDeviceEvent::InterruptRequested) {
        return Err(verification_error(
            "MMIO device interrupt event",
            "accepted device write did not publish InterruptRequested",
        ));
    }
    if mmio.take_device_event().is_some() {
        return Err(verification_error(
            "MMIO device interrupt event ownership",
            "one device write published more than one consumable interrupt event",
        ));
    }

    vm.pulse_gsi_edge(KvmBackend::IRQCHIP_GSI)?;

    let handler_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        MMIO_INTERRUPT_HANDLER_BYTE,
        "MMIO device interrupt handler",
    )?;
    let resumed_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        MMIO_INTERRUPT_RESUMED_BYTE,
        "MMIO device interrupt resumed main",
    )?;
    let completion_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        MMIO_INTERRUPT_DONE_BYTE,
        "MMIO device interrupt completion barrier",
    )?;
    let completion = vcpu.registers()?;
    require_interrupt_enabled_flags("MMIO device interrupt completion state", completion.rflags)?;

    let io_exits = vec![handler_io, resumed_io, completion_io];
    let writes = mmio.writes().unwrap_or(&[]).to_vec();
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if writes.as_slice() != [MMIO_INTERRUPT_WRITE_VALUE]
        || proof.as_slice() != MMIO_INTERRUPT_PROOF
        || io_exits.len() != MMIO_INTERRUPT_PROOF.len()
    {
        return Err(verification_error(
            "MMIO device interrupt execution proof",
            format!(
                "expected write {:?} and proof {:?}; got writes {:?} and proof {:?}",
                [MMIO_INTERRUPT_WRITE_VALUE],
                MMIO_INTERRUPT_PROOF,
                writes,
                proof
            ),
        ));
    }

    Ok(LongModeMmioInterruptGuestResult {
        gsi: KvmBackend::IRQCHIP_GSI,
        vector: KvmBackend::IRQCHIP_VECTOR,
        lapic_spiv: lapic.spiv(),
        lapic_lint0: lapic.lint0(),
        event_rflags: event_registers.rflags,
        completion_rflags: completion.rflags,
        device_event_count: 1,
        mmio_exit,
        io_exits,
        writes,
        proof,
    })
}

fn validate_trigger_mmio(exit: &MmioExit) -> Result<(), Error> {
    if exit.direction() != MmioDirection::Write
        || exit.address() != LONG_MODE_MMIO_DEVICE_GPA
        || exit.length() != 1
        || exit.write_data() != [MMIO_INTERRUPT_WRITE_VALUE]
    {
        return Err(verification_error(
            "MMIO device interrupt trigger",
            format!(
                "expected one-byte write {:?} to GPA {:#x}, got direction {:?}, address {:#x}, length {}, data {:?}",
                [MMIO_INTERRUPT_WRITE_VALUE],
                LONG_MODE_MMIO_DEVICE_GPA,
                exit.direction(),
                exit.address(),
                exit.length(),
                exit.write_data()
            ),
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
                "expected byte-wide debug-port output {:?}, got direction {:?}, size {}, port {:#x}, count {}, data {:?}",
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
            "debug output exit unexpectedly requested an input response",
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
            format!(
                "expected architectural RFLAGS bit 1 and IF set, got RFLAGS {rflags:#x}"
            ),
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
    use crate::interrupt::LONG_MODE_INTERRUPT_GDT_ADDR;
    use crate::long_mode::LONG_MODE_ALIAS_PT_ADDR;

    fn read_u64(memory: &GuestMemory, address: GuestPhysAddr) -> u64 {
        let mut bytes = [0_u8; 8];
        memory.read(address, &mut bytes).unwrap();
        u64::from_le_bytes(bytes)
    }

    #[test]
    fn combined_layout_preserves_virtual_mmio_pte_and_interrupt_gate() {
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE).unwrap();
        let layout = LongModeMmioInterruptLayout::new(
            memory.region(),
            LONG_MODE_INTERRUPT_GUEST_ENTRY,
            LONG_MODE_MMIO_STACK_POINTER,
            LONG_MODE_INTERRUPT_VECTOR,
            LONG_MODE_INTERRUPT_HANDLER,
        )
        .unwrap();
        layout.install_tables(&mut memory).unwrap();

        let pte_index =
            (LONG_MODE_MMIO_VIRTUAL_PAGE - LONG_MODE_ALIAS_VIRTUAL_BASE) / LONG_MODE_PAGE_SIZE;
        assert_eq!(
            read_u64(
                &memory,
                GuestPhysAddr::new(LONG_MODE_ALIAS_PT_ADDR.get() + pte_index * 8)
            ),
            LONG_MODE_MMIO_DEVICE_GPA | 0x3
        );

        let gate_address = GuestPhysAddr::new(
            LONG_MODE_INTERRUPT_IDT_ADDR.get()
                + u64::from(LONG_MODE_INTERRUPT_VECTOR) * INTERRUPT_GATE_SIZE,
        );
        let mut gate = [0_u8; 16];
        memory.read(gate_address, &mut gate).unwrap();
        assert_eq!(u16::from_le_bytes([gate[2], gate[3]]), 0x8);
        assert_eq!(gate[5], 0x8e);
        let target = u64::from(u16::from_le_bytes([gate[0], gate[1]]))
            | (u64::from(u16::from_le_bytes([gate[6], gate[7]])) << 16)
            | (u64::from(u32::from_le_bytes([gate[8], gate[9], gate[10], gate[11]])) << 32);
        assert_eq!(target, LONG_MODE_INTERRUPT_HANDLER.get());

        let mut gdt = [0_u8; 24];
        memory.read(LONG_MODE_INTERRUPT_GDT_ADDR, &mut gdt).unwrap();
        assert_ne!(gdt, [0; 24]);
    }

    #[test]
    fn fixture_machine_code_and_proof_are_stable() {
        assert_eq!(MMIO_INTERRUPT_GUEST_BYTES.len(), 61);
        assert_eq!(&MMIO_INTERRUPT_GUEST_BYTES[39..49], &[0x48, 0xbb, 0, 0, 0x50, 0, 0, 0, 0, 0]);
        assert_eq!(&MMIO_INTERRUPT_GUEST_BYTES[49..52], &[0xc6, 0x03, b'W']);
        assert_eq!(&MMIO_INTERRUPT_GUEST_BYTES[52..56], &[0xb0, b'M', 0xe6, 0xe9]);
        assert_eq!(&MMIO_INTERRUPT_GUEST_BYTES[56..60], &[0xb0, b'D', 0xe6, 0xe9]);
        assert_eq!(MMIO_INTERRUPT_GUEST_BYTES[60], 0xf4);
        assert_eq!(MMIO_INTERRUPT_HANDLER_BYTES.len(), 10);
        assert_eq!(MMIO_INTERRUPT_PROOF, b"IMD");
    }
}
