use crate::config::VmConfig;
use crate::error::{Error, HostEnvironmentError, KvmCapabilityError, VmExitError};
use crate::execution::run_vcpu_until_stopped;
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
use crate::vmexit::VmExitReport;
use std::os::fd::AsRawFd;

const KVM_CAP_IRQCHIP: i32 = 0;
const KVM_CREATE_IRQCHIP: libc::c_ulong = 0xAE60;
const KVM_IRQ_LINE: libc::c_ulong = 0x4008_AE61;
const IRQCHIP_POST_PULSE_EXIT_BUDGET: u32 = 3;
const X86_RFLAGS_RESERVED_BIT: u64 = 1 << 1;

const IRQCHIP_GUEST_BYTES: [u8; 49] = [
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
    0xb0, b'R', 0xe6, 0xe9, // readiness proof; the following KVM_RUN commits this I/O
    0xf4, // hlt -- stable armed wait point; GSI0 must wake this vCPU
    0xb0, b'M', 0xe6, 0xe9, // resumed-main proof after interrupt + IRETQ
    0xf4, // terminal hlt
];

const IRQCHIP_HANDLER_BYTES: [u8; 10] = [
    0xb0, b'I', 0xe6, 0xe9, // interrupt-handler proof
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrqchipGuestResult {
    gsi: u32,
    vector: u8,
    readiness_rip: u64,
    readiness_rflags: u64,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
    report: VmExitReport,
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
    pub const fn readiness_rip(&self) -> u64 {
        self.readiness_rip
    }

    #[must_use]
    pub const fn readiness_rflags(&self) -> u64 {
        self.readiness_rflags
    }

    #[must_use]
    pub fn io_exits(&self) -> &[PortIoExit] {
        &self.io_exits
    }

    #[must_use]
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    #[must_use]
    pub const fn report(&self) -> VmExitReport {
        self.report
    }
}

impl KvmBackend {
    pub const IRQCHIP_GSI: u32 = 0;
    pub const IRQCHIP_VECTOR: u8 = LONG_MODE_INTERRUPT_VECTOR;
    pub const IRQCHIP_PROOF: &'static [u8; 3] = b"RIM";
    pub const IRQCHIP_READY_RIP: u64 = 0x1_002c;
    pub const IRQCHIP_TERMINAL_RIP: u64 = 0x1_0031;

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
        let mut port_io = PortIoBus::with_debug_port();

        let readiness_output_exit = vcpu.run_once()?;
        if readiness_output_exit != VcpuExit::Io {
            return Err(Error::VmExit(VmExitError::UnexpectedSequence {
                stage: "irqchip readiness output",
                expected_reason: VcpuExit::Io.reason(),
                actual_reason: readiness_output_exit.reason(),
            }));
        }
        let readiness_io = vcpu.port_io_exit()?;
        validate_readiness_io(&readiness_io)?;
        if port_io.dispatch(&readiness_io)? != PortIoService::Output {
            return Err(verification_error(
                "irqchip readiness output",
                "readiness exit unexpectedly requested an input response",
            ));
        }

        // A serviceable KVM_EXIT_IO is not a portable architectural commit point: on some KVM
        // implementations KVM_GET_REGS still reports RIP at the I/O instruction until the next
        // KVM_RUN completes that exit. Re-enter once with no GSI asserted; the guest completes the
        // readiness OUT and reaches this dedicated HLT. Only that committed halted state is used as
        // the host-visible arm point for the edge-triggered GSI proof.
        let armed_exit = vcpu.run_once()?;
        if armed_exit != VcpuExit::Hlt {
            return Err(Error::VmExit(VmExitError::UnexpectedSequence {
                stage: "irqchip armed wait",
                expected_reason: VcpuExit::Hlt.reason(),
                actual_reason: armed_exit.reason(),
            }));
        }

        let readiness = vcpu.registers()?;
        if readiness.rip != Self::IRQCHIP_READY_RIP
            || readiness.rflags & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
            || readiness.rflags & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
        {
            return Err(verification_error(
                "irqchip armed wait state",
                format!(
                    "expected committed HLT RIP {:#x} with reserved RFLAGS bit and IF set, got RIP {:#x}, RFLAGS {:#x}",
                    Self::IRQCHIP_READY_RIP,
                    readiness.rip,
                    readiness.rflags
                ),
            ));
        }

        vm.pulse_gsi_edge(Self::IRQCHIP_GSI)?;
        let execution = run_vcpu_until_stopped(
            &mut vcpu,
            &mut port_io,
            IRQCHIP_POST_PULSE_EXIT_BUDGET,
        )?;

        let mut io_exits = Vec::with_capacity(Self::IRQCHIP_PROOF.len());
        io_exits.push(readiness_io);
        io_exits.extend_from_slice(execution.io_exits());
        let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
        let report = execution.report();

        if proof.as_slice() != Self::IRQCHIP_PROOF
            || io_exits.len() != Self::IRQCHIP_PROOF.len()
            || report.exit() != VcpuExit::Hlt
            || report.rip() != Self::IRQCHIP_TERMINAL_RIP
            || report.rflags() & X86_RFLAGS_RESERVED_BIT != X86_RFLAGS_RESERVED_BIT
            || report.rflags() & X86_RFLAGS_INTERRUPT_ENABLE != X86_RFLAGS_INTERRUPT_ENABLE
        {
            return Err(verification_error(
                "irqchip GSI execution proof",
                format!(
                    "expected proof {:?}, HLT RIP {:#x}, reserved RFLAGS bit and IF set; got proof {:?}, exit {:?}, RIP {:#x}, RFLAGS {:#x}",
                    Self::IRQCHIP_PROOF,
                    Self::IRQCHIP_TERMINAL_RIP,
                    proof,
                    report.exit(),
                    report.rip(),
                    report.rflags()
                ),
            ));
        }

        Ok(IrqchipGuestResult {
            gsi: Self::IRQCHIP_GSI,
            vector: Self::IRQCHIP_VECTOR,
            readiness_rip: readiness.rip,
            readiness_rflags: readiness.rflags,
            io_exits,
            proof,
            report,
        })
    }
}

impl Vm {
    pub fn pulse_gsi_edge(&self, gsi: u32) -> Result<(), Error> {
        set_irq_line(self.fd.as_raw_fd(), KvmIrqLevel::new(gsi, true)).map_err(|source| {
            Error::HostEnvironment(HostEnvironmentError::VmOperation {
                operation: "KVM_IRQ_LINE assert",
                source,
            })
        })?;
        set_irq_line(self.fd.as_raw_fd(), KvmIrqLevel::new(gsi, false)).map_err(|source| {
            Error::HostEnvironment(HostEnvironmentError::VmOperation {
                operation: "KVM_IRQ_LINE deassert",
                source,
            })
        })
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

fn validate_readiness_io(io_exit: &PortIoExit) -> Result<(), Error> {
    if io_exit.direction() != PortIoDirection::Out
        || io_exit.size() != 1
        || io_exit.port() != DEBUG_PORT
        || io_exit.count() != 1
        || io_exit.output_data() != b"R"
    {
        return Err(verification_error(
            "irqchip readiness output",
            format!(
                "expected byte-wide debug-port output R, got direction {:?}, size {}, port {:#x}, count {}, data {:?}",
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

    #[test]
    fn irqchip_uapi_contract_matches_x86_kvm() {
        assert_eq!(KVM_CAP_IRQCHIP, 0);
        assert_eq!(KVM_CREATE_IRQCHIP, 0xAE60);
        assert_eq!(KVM_IRQ_LINE, 0x4008_AE61);
        assert_eq!(std::mem::size_of::<KvmIrqLevel>(), 8);
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
        assert_eq!(IRQCHIP_GUEST_BYTES.len(), 49);
        assert_eq!(IRQCHIP_GUEST_BYTES[43], 0xf4);
        assert_eq!(IRQCHIP_HANDLER_BYTES.len(), 10);
        assert_eq!(KvmBackend::IRQCHIP_READY_RIP, 0x1_002c);
        assert_eq!(KvmBackend::IRQCHIP_TERMINAL_RIP, 0x1_0031);
        assert_eq!(KvmBackend::IRQCHIP_PROOF, b"RIM");
        assert_eq!(KvmBackend::IRQCHIP_GSI, 0);
        assert_eq!(KvmBackend::IRQCHIP_VECTOR, 0x40);
    }
}
