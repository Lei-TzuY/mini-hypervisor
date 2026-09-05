use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError, KvmCapabilityError, VmExitError};
use crate::interrupt::{
    LongModeInterruptLayout, LONG_MODE_INTERRUPT_GUEST_ENTRY, LONG_MODE_INTERRUPT_HANDLER,
    LONG_MODE_INTERRUPT_STACK_POINTER, LONG_MODE_INTERRUPT_VECTOR, X86_RFLAGS_INTERRUPT_ENABLE,
};
use crate::kvm::{KvmBackend, Vm};
use crate::loader::FlatGuestImage;
use crate::long_mode::LONG_MODE_IDENTITY_MAP_SIZE;
use crate::memory::{GuestMemory, GuestPhysAddr};
use crate::portio::{PortIoBus, PortIoService, DEBUG_PORT};
use crate::vcpu::{PortIoDirection, PortIoExit, VcpuExit, VcpuId};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

const KVM_CAP_IRQCHIP: i32 = 0;
const KVM_CREATE_IRQCHIP: libc::c_ulong = 0xAE60;
const KVM_IRQ_LINE: libc::c_ulong = 0x4008_AE61;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;
const IRQCHIP_READY_BYTE: u8 = b'R';
const IRQCHIP_ARMED_BYTE: u8 = b'A';
const IRQCHIP_HANDLER_BYTE: u8 = b'I';
const IRQCHIP_RESUMED_BYTE: u8 = b'M';
const IRQCHIP_DONE_BYTE: u8 = b'D';

const IRQCHIP_GUEST_BYTES: [u8; 56] = [
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
    0xb0, IRQCHIP_READY_BYTE, 0xe6, 0xe9, // first readiness output
    0xb0, IRQCHIP_ARMED_BYTE, 0xe6, 0xe9, // second I/O barrier; R is committed here
    0xb0, IRQCHIP_RESUMED_BYTE, 0xe6, 0xe9, // resumed-main proof after interrupt + IRETQ
    0xb0, IRQCHIP_DONE_BYTE, 0xe6, 0xe9, // completion barrier; M is committed here
    0xf4, // safety fallback; host deliberately does not re-enter after D
];

const IRQCHIP_HANDLER_BYTES: [u8; 10] = [
    0xb0, IRQCHIP_HANDLER_BYTE, 0xe6, 0xe9, // interrupt-handler proof
    0xb0, 0x20, 0xe6, 0x20, // non-specific EOI to the master PIC
    0x48, 0xcf, // iretq
];

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KvmIrqLevel {
    irq: u32,
    level: u32,
}

impl KvmIrqLevel {
    const fn new(irq: u32, level: bool) -> Self {
        Self {
            irq,
            level: level as u32,
        }
    }
}

#[derive(Debug)]
pub(crate) struct VmIrqLineHandle {
    fd: OwnedFd,
}

impl VmIrqLineHandle {
    pub(crate) fn set_gsi_level(&self, gsi: u32, asserted: bool) -> io::Result<()> {
        set_irq_line(self.fd.as_raw_fd(), KvmIrqLevel::new(gsi, asserted))
    }

    pub(crate) fn pulse_gsi_edge(&self, gsi: u32) -> io::Result<()> {
        self.set_gsi_level(gsi, true)?;
        self.set_gsi_level(gsi, false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrqchipGuestResult {
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    armed_rflags: u64,
    completion_rflags: u64,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
}

impl IrqchipGuestResult {
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
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
}

impl KvmBackend {
    pub const IRQCHIP_GSI: u32 = 0;
    pub const IRQCHIP_VECTOR: u8 = LONG_MODE_INTERRUPT_VECTOR;
    pub const IRQCHIP_PROOF: &'static [u8; 5] = b"RAIMD";

    pub fn create_vm_with_irqchip(&self) -> Result<Vm, Error> {
        require_irqchip_capability(self)?;
        let vm = self.create_vm()?;
        ioctl_noarg(vm.fd.as_raw_fd(), KVM_CREATE_IRQCHIP).map_err(|source| {
            Error::HostEnvironment(HostEnvironmentError::VmOperation {
                operation: "KVM_CREATE_IRQCHIP",
                source,
            })
        })?;
        Ok(vm)
    }

    pub fn run_irqchip_gsi_guest(config: VmConfig) -> Result<IrqchipGuestResult, Error> {
        let guest = FlatGuestImage::new(
            LONG_MODE_INTERRUPT_GUEST_ENTRY,
            LONG_MODE_INTERRUPT_GUEST_ENTRY,
            &IRQCHIP_GUEST_BYTES,
        )?;
        let handler = FlatGuestImage::new(
            LONG_MODE_INTERRUPT_HANDLER,
            LONG_MODE_INTERRUPT_HANDLER,
            &IRQCHIP_HANDLER_BYTES,
        )?;

        let backend = Self::open()?;
        let mut vm = backend.create_vm_with_irqchip()?;
        let mut memory =
            GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
        let layout = LongModeInterruptLayout::new(
            memory.region(),
            guest.entry(),
            LONG_MODE_INTERRUPT_STACK_POINTER,
            Self::IRQCHIP_VECTOR,
            handler.entry(),
        )
        .expect("fixed deterministic irqchip fixture layout remains valid");
        layout.install_tables(&mut memory)?;
        guest.load(&mut memory)?;
        handler.load(&mut memory)?;
        vm.register_guest_memory(memory)?;

        debug_assert_eq!(config.vcpu_count(), 1);
        let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
        vcpu.initialize_long_mode_interrupts(&layout)?;
        let lapic = vcpu.configure_legacy_pic_extint()?;
        let mut port_io = PortIoBus::with_debug_port();

        let readiness_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IRQCHIP_READY_BYTE,
            "irqchip readiness output",
        )?;

        // Re-entering KVM_RUN to reach a second I/O exit commits the preceding R output on every
        // supported KVM implementation. A serviceable KVM_EXIT_IO is not itself a portable RIP
        // commit point, so this fixture never assigns architectural meaning to the RIP observed at
        // either output exit. The second A output is the explicit userspace barrier: only after it
        // has been observed and guest IF is verified do we assert the GSI edge.
        let armed_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IRQCHIP_ARMED_BYTE,
            "irqchip armed barrier",
        )?;
        let armed = vcpu.registers()?;
        require_interrupt_enabled_flags("irqchip armed barrier state", armed.rflags)?;

        vm.pulse_gsi_edge(Self::IRQCHIP_GSI)?;

        let handler_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IRQCHIP_HANDLER_BYTE,
            "irqchip handler output",
        )?;
        let resumed_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IRQCHIP_RESUMED_BYTE,
            "irqchip resumed-main output",
        )?;
        let completion_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IRQCHIP_DONE_BYTE,
            "irqchip completion barrier",
        )?;
        let completion = vcpu.registers()?;
        require_interrupt_enabled_flags("irqchip completion state", completion.rflags)?;

        // With an in-kernel local APIC, x86 KVM keeps a guest HLT inside the kernel as a
        // non-runnable vCPU until a wake event arrives instead of guaranteeing KVM_EXIT_HLT.
        // Observing D after M is therefore the terminal userspace synchronization point. Reaching
        // D necessarily required one more KVM_RUN after the M exit, so the resumed-main M output
        // is committed without depending on non-portable serviceable-I/O RIP semantics.
        let io_exits = vec![
            readiness_io,
            armed_io,
            handler_io,
            resumed_io,
            completion_io,
        ];
        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();

        if proof.as_slice() != Self::IRQCHIP_PROOF || io_exits.len() != Self::IRQCHIP_PROOF.len() {
            return Err(verification_error(
                "irqchip GSI execution proof",
                format!(
                    "expected exact proof {:?} across {} byte-wide I/O exits, got proof {:?} across {} exits",
                    Self::IRQCHIP_PROOF,
                    Self::IRQCHIP_PROOF.len(),
                    proof,
                    io_exits.len()
                ),
            ));
        }

        Ok(IrqchipGuestResult {
            gsi: Self::IRQCHIP_GSI,
            vector: Self::IRQCHIP_VECTOR,
            lapic_spiv: lapic.spiv(),
            lapic_lint0: lapic.lint0(),
            armed_rflags: armed.rflags,
            completion_rflags: completion.rflags,
            io_exits,
            proof,
        })
    }
}

impl Vm {
    pub(crate) fn duplicate_irq_line_handle(&self) -> io::Result<VmIrqLineHandle> {
        // SAFETY: `dup` only duplicates this process-owned KVM VM file descriptor. The returned
        // descriptor refers to the same kernel VM object and is immediately wrapped in `OwnedFd`.
        let raw_fd = unsafe { libc::dup(self.fd.as_raw_fd()) };
        if raw_fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful `dup` returns a fresh owned descriptor that must be closed exactly once.
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        Ok(VmIrqLineHandle { fd })
    }

    pub fn set_gsi_level(&self, gsi: u32, asserted: bool) -> Result<(), Error> {
        let operation = if asserted {
            "KVM_IRQ_LINE assert"
        } else {
            "KVM_IRQ_LINE deassert"
        };
        set_irq_line(self.fd.as_raw_fd(), KvmIrqLevel::new(gsi, asserted)).map_err(|source| {
            Error::HostEnvironment(HostEnvironmentError::VmOperation { operation, source })
        })
    }

    pub fn pulse_gsi_edge(&self, gsi: u32) -> Result<(), Error> {
        self.set_gsi_level(gsi, true)?;
        self.set_gsi_level(gsi, false)
    }
}

fn require_irqchip_capability(backend: &KvmBackend) -> Result<(), Error> {
    let capability = libc::c_ulong::try_from(KVM_CAP_IRQCHIP)
        .expect("KVM_CAP_IRQCHIP is a non-negative capability ID");
    let value = ioctl_with_arg(backend.fd.as_raw_fd(), KVM_CHECK_EXTENSION, capability)
        .map_err(|source| {
            Error::HostEnvironment(HostEnvironmentError::Io {
                operation: "KVM_CHECK_EXTENSION KVM_CAP_IRQCHIP",
                source,
            })
        })?;
    if value <= 0 {
        return Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
            name: "KVM_CAP_IRQCHIP",
            id: KVM_CAP_IRQCHIP,
        }));
    }
    Ok(())
}

fn set_irq_line(fd: std::os::fd::RawFd, request: KvmIrqLevel) -> io::Result<()> {
    // SAFETY: `request` is the fixed eight-byte `struct kvm_irq_level` and remains readable for
    // the duration of the VM ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_IRQ_LINE, &request) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn run_expected_debug_output(
    vcpu: &mut crate::vcpu::Vcpu,
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
    validate_debug_output(&io_exit, expected, stage)?;
    if port_io.dispatch(&io_exit)? != PortIoService::Output {
        return Err(verification_error(
            stage,
            "debug output exit unexpectedly requested an input response",
        ));
    }
    Ok(io_exit)
}

fn validate_debug_output(
    io_exit: &PortIoExit,
    expected: u8,
    stage: &'static str,
) -> Result<(), Error> {
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
    Ok(())
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

const _: () = {
    assert!(std::mem::size_of::<KvmIrqLevel>() == 8);
    assert!(std::mem::size_of::<KvmRunIoPrefix>() >= 48);
};

#[cfg(test)]
mod irqchip_tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn irqchip_uapi_contract_matches_x86_kvm() {
        assert_eq!(KVM_CAP_IRQCHIP, 0);
        assert_eq!(KVM_CREATE_IRQCHIP, 0xAE60);
        assert_eq!(KVM_IRQ_LINE, 0x4008_AE61);
        assert_eq!(std::mem::size_of::<KvmIrqLevel>(), 8);
    }

    #[test]
    fn irq_line_worker_handle_is_send_without_sharing_vm_memory() {
        assert_send::<VmIrqLineHandle>();
    }

    #[test]
    fn irq_line_edge_requests_preserve_gsi_and_binary_levels() {
        assert_eq!(
            KvmIrqLevel::new(7, true),
            KvmIrqLevel { irq: 7, level: 1 }
        );
        assert_eq!(
            KvmIrqLevel::new(7, false),
            KvmIrqLevel { irq: 7, level: 0 }
        );
    }

    #[test]
    fn deterministic_irqchip_guest_and_handler_bytes_are_stable() {
        assert_eq!(IRQCHIP_GUEST_BYTES.len(), 56);
        assert_eq!(&IRQCHIP_GUEST_BYTES[39..43], &[0xb0, b'R', 0xe6, 0xe9]);
        assert_eq!(&IRQCHIP_GUEST_BYTES[43..47], &[0xb0, b'A', 0xe6, 0xe9]);
        assert_eq!(&IRQCHIP_GUEST_BYTES[47..51], &[0xb0, b'M', 0xe6, 0xe9]);
        assert_eq!(&IRQCHIP_GUEST_BYTES[51..55], &[0xb0, b'D', 0xe6, 0xe9]);
        assert_eq!(IRQCHIP_GUEST_BYTES[55], 0xf4);
        assert_eq!(IRQCHIP_HANDLER_BYTES.len(), 10);
        assert_eq!(KvmBackend::IRQCHIP_PROOF, b"RAIMD");
        assert_eq!(KvmBackend::IRQCHIP_GSI, 0);
        assert_eq!(KvmBackend::IRQCHIP_VECTOR, 0x40);
    }

    #[test]
    fn interrupt_enabled_flag_contract_requires_reserved_bit_and_if() {
        assert!(require_interrupt_enabled_flags("test", 0x202).is_ok());
        assert!(require_interrupt_enabled_flags("test", 0x200).is_err());
        assert!(require_interrupt_enabled_flags("test", 0x002).is_err());
    }
}

include!("async_timer.rs");
