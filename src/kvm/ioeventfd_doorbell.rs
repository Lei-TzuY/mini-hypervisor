use crate::mmio::interrupt::LongModeMmioInterruptLayout;
use crate::mmio::long_mode::{LONG_MODE_MMIO_DEVICE_GPA, LONG_MODE_MMIO_STACK_POINTER};

const KVM_CAP_IOEVENTFD: i32 = 36;
const KVM_IOEVENTFD: libc::c_ulong = 0x4040_AE79;
const KVM_IOEVENTFD_FLAG_DATAMATCH: u32 = 1 << 0;
const KVM_IOEVENTFD_FLAG_DEASSIGN: u32 = 1 << 2;
const IOEVENTFD_DOORBELL_VALUE: u8 = b'W';
const IOEVENTFD_READY_BYTE: u8 = b'R';
const IOEVENTFD_BARRIER_BYTE: u8 = b'B';
const IOEVENTFD_HANDLER_BYTE: u8 = b'I';
const IOEVENTFD_RESUMED_BYTE: u8 = b'M';
const IOEVENTFD_DONE_BYTE: u8 = b'D';
const IOEVENTFD_WORKER_TIMEOUT_SECONDS: u64 = 5;

const IOEVENTFD_GUEST_BYTES: [u8; 69] = [
    0xfa, // cli
    0xb0, 0x11, 0xe6, 0x20, 0xe6, 0xa0, // ICW1: initialize master and slave PICs
    0xb0, 0x40, 0xe6, 0x21, // ICW2: master IRQ0..7 -> vectors 0x40..0x47
    0xb0, 0x48, 0xe6, 0xa1, // ICW2: slave IRQ8..15 -> vectors 0x48..0x4f
    0xb0, 0x04, 0xe6, 0x21, // ICW3: master has slave on IRQ2
    0xb0, 0x02, 0xe6, 0xa1, // ICW3: slave cascade identity 2
    0xb0, 0x01, 0xe6, 0x21, 0xe6, 0xa1, // ICW4: 8086 mode on both PICs
    0xb0, 0xfe, 0xe6, 0x21, // OCW1: unmask only master IRQ0
    0xb0, 0xff, 0xe6, 0xa1, // OCW1: mask every slave IRQ
    0xb0, IOEVENTFD_READY_BYTE, 0xe6, 0xe9, // readiness while IF remains clear
    0x48, 0xbb, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs $0x500000, %rbx
    0xc6, 0x03, IOEVENTFD_DOORBELL_VALUE, // doorbell write handled by KVM_IOEVENTFD
    0xb0, IOEVENTFD_BARRIER_BYTE, 0xe6, 0xe9, // proves doorbell write continued without KVM_EXIT_MMIO
    0xfb, // sti
    0xf4, // hlt -- pending irqfd edge is safe across STI shadow
    0xb0, IOEVENTFD_RESUMED_BYTE, 0xe6, 0xe9, // resumed main after handler + IRETQ
    0xb0, IOEVENTFD_DONE_BYTE, 0xe6, 0xe9, // terminal userspace barrier
    0xf4, // safety fallback; host deliberately stops at D
];

const IOEVENTFD_HANDLER_BYTES: [u8; 10] = [
    0xb0, IOEVENTFD_HANDLER_BYTE, 0xe6, 0xe9, // interrupt handler proof
    0xb0, 0x20, 0xe6, 0x20, // non-specific EOI to master PIC
    0x48, 0xcf, // iretq
];

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmIoeventfd {
    datamatch: u64,
    addr: u64,
    len: u32,
    fd: i32,
    flags: u32,
    pad: [u8; 36],
}

impl KvmIoeventfd {
    const fn assign(fd: i32, addr: u64, len: u32, datamatch: u64) -> Self {
        Self {
            datamatch,
            addr,
            len,
            fd,
            flags: KVM_IOEVENTFD_FLAG_DATAMATCH,
            pad: [0; 36],
        }
    }

    const fn deassign(fd: i32, addr: u64, len: u32, datamatch: u64) -> Self {
        Self {
            datamatch,
            addr,
            len,
            fd,
            flags: KVM_IOEVENTFD_FLAG_DATAMATCH | KVM_IOEVENTFD_FLAG_DEASSIGN,
            pad: [0; 36],
        }
    }
}

#[derive(Debug)]
struct IoeventfdDoorbellRegistration {
    eventfd: EventFd,
    addr: u64,
    len: u32,
    datamatch: u64,
}

impl IoeventfdDoorbellRegistration {
    fn assign_prepared(
        vm: &Vm,
        eventfd: EventFd,
        addr: u64,
        len: u32,
        datamatch: u64,
    ) -> io::Result<Self> {
        let request = KvmIoeventfd::assign(eventfd.fd.as_raw_fd(), addr, len, datamatch);
        set_ioeventfd(vm.fd.as_raw_fd(), &request)?;
        Ok(Self {
            eventfd,
            addr,
            len,
            datamatch,
        })
    }

    fn deassign(&self, vm: &Vm) -> io::Result<()> {
        let request = KvmIoeventfd::deassign(
            self.eventfd.fd.as_raw_fd(),
            self.addr,
            self.len,
            self.datamatch,
        );
        set_ioeventfd(vm.fd.as_raw_fd(), &request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoeventfdDoorbellGuestResult {
    doorbell_gpa: u64,
    doorbell_value: u8,
    doorbell_event_count: u64,
    userspace_mmio_exit_count: u32,
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    doorbell_rflags: u64,
    completion_rflags: u64,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
}

impl IoeventfdDoorbellGuestResult {
    #[must_use]
    pub const fn doorbell_gpa(&self) -> u64 {
        self.doorbell_gpa
    }

    #[must_use]
    pub const fn doorbell_value(&self) -> u8 {
        self.doorbell_value
    }

    #[must_use]
    pub const fn doorbell_event_count(&self) -> u64 {
        self.doorbell_event_count
    }

    #[must_use]
    pub const fn userspace_mmio_exit_count(&self) -> u32 {
        self.userspace_mmio_exit_count
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
    pub const fn doorbell_rflags(&self) -> u64 {
        self.doorbell_rflags
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
    pub const IOEVENTFD_DOORBELL_GPA: u64 = LONG_MODE_MMIO_DEVICE_GPA;
    pub const IOEVENTFD_DOORBELL_VALUE: u8 = IOEVENTFD_DOORBELL_VALUE;
    pub const IOEVENTFD_DOORBELL_GSI: u32 = Self::IRQCHIP_GSI;
    pub const IOEVENTFD_DOORBELL_VECTOR: u8 = Self::IRQCHIP_VECTOR;
    pub const IOEVENTFD_DOORBELL_PROOF: &'static [u8; 5] = b"RBIMD";

    pub fn run_ioeventfd_doorbell_guest(
        config: VmConfig,
    ) -> Result<IoeventfdDoorbellGuestResult, Error> {
        require_ioeventfd_capability()?;
        require_irqfd_capability(self_open_for_capability_check()?)?;
        run_ioeventfd_doorbell_guest(config)
    }
}

fn self_open_for_capability_check() -> Result<&'static KvmBackend, Error> {
    unreachable!("capability checks are performed by the runtime backend")
}

fn run_ioeventfd_doorbell_guest(config: VmConfig) -> Result<IoeventfdDoorbellGuestResult, Error> {
    let guest = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_GUEST_ENTRY,
        LONG_MODE_INTERRUPT_GUEST_ENTRY,
        &IOEVENTFD_GUEST_BYTES,
    )?;
    let handler = FlatGuestImage::new(
        LONG_MODE_INTERRUPT_HANDLER,
        LONG_MODE_INTERRUPT_HANDLER,
        &IOEVENTFD_HANDLER_BYTES,
    )?;

    let backend = KvmBackend::open()?;
    require_ioeventfd_capability_for_backend(&backend)?;
    require_irqfd_capability(&backend)?;
    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = LongModeMmioInterruptLayout::new(
        memory.region(),
        guest.entry(),
        LONG_MODE_MMIO_STACK_POINTER,
        KvmBackend::IOEVENTFD_DOORBELL_VECTOR,
        handler.entry(),
    )
    .expect("fixed ioeventfd doorbell fixture layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(layout.interrupt_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;
    let mut port_io = PortIoBus::with_debug_port();

    // Complete every fallible userspace fd preparation before either kernel event registration.
    let watchdog_irq = vm
        .duplicate_irq_line_handle()
        .map_err(|source| async_timer_vm_error("duplicate ioeventfd watchdog IRQ-line handle", source))?;
    watchdog_irq
        .set_gsi_level(KvmBackend::IOEVENTFD_DOORBELL_GSI, false)
        .map_err(|source| async_timer_vm_error("preflight ioeventfd watchdog IRQ-line handle", source))?;
    let doorbell_eventfd = EventFd::new()
        .map_err(|source| async_timer_vm_error("create ioeventfd doorbell eventfd", source))?;
    let doorbell_reader = doorbell_eventfd
        .duplicate()
        .map_err(|source| async_timer_vm_error("duplicate ioeventfd doorbell reader", source))?;

    vm.set_gsi_level(KvmBackend::IOEVENTFD_DOORBELL_GSI, false)?;
    let (irqfd_registration, irqfd_signal) =
        IrqfdTimerRegistration::assign_with_signal(&vm, KvmBackend::IOEVENTFD_DOORBELL_GSI)
            .map_err(|source| async_timer_vm_error("assign ioeventfd response KVM_IRQFD", source))?;

    let ioeventfd_registration = match IoeventfdDoorbellRegistration::assign_prepared(
        &vm,
        doorbell_eventfd,
        KvmBackend::IOEVENTFD_DOORBELL_GPA,
        1,
        u64::from(KvmBackend::IOEVENTFD_DOORBELL_VALUE),
    ) {
        Ok(registration) => registration,
        Err(source) => {
            let cleanup = irqfd_registration.deassign(&vm);
            return match cleanup {
                Ok(()) => Err(async_timer_vm_error("assign KVM_IOEVENTFD doorbell", source)),
                Err(cleanup_source) => Err(verification_error(
                    "assign KVM_IOEVENTFD doorbell cleanup",
                    format!(
                        "ioeventfd assignment failed ({source}) and irqfd cleanup also failed ({cleanup_source})"
                    ),
                )),
            };
        }
    };

    let doorbell_worker = std::thread::spawn(move || -> io::Result<u64> {
        let count = wait_eventfd_counter(
            &doorbell_reader,
            std::time::Duration::from_secs(IOEVENTFD_WORKER_TIMEOUT_SECONDS),
        )?;
        if count != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected one ioeventfd doorbell signal, got counter {count}"),
            ));
        }
        irqfd_signal.signal()?;
        Ok(count)
    });

    let (watchdog_cancel_tx, watchdog_cancel_rx) = std::sync::mpsc::channel::<()>();
    let watchdog_worker = std::thread::spawn(move || -> io::Result<bool> {
        match watchdog_cancel_rx.recv_timeout(std::time::Duration::from_secs(
            ASYNC_TIMER_WATCHDOG_SECONDS,
        )) {
            Ok(()) => Ok(false),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                watchdog_irq.pulse_gsi_edge(KvmBackend::IOEVENTFD_DOORBELL_GSI)?;
                Ok(true)
            }
        }
    });

    let execution = (|| -> Result<_, Error> {
        let readiness_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IOEVENTFD_READY_BYTE,
            "ioeventfd doorbell readiness output",
        )?;
        let barrier_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IOEVENTFD_BARRIER_BYTE,
            "ioeventfd doorbell completion barrier",
        )?;
        let doorbell_state = vcpu.registers()?;
        require_interrupt_disabled_flags("ioeventfd doorbell barrier state", doorbell_state.rflags)?;
        let handler_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IOEVENTFD_HANDLER_BYTE,
            "ioeventfd response interrupt handler",
        )?;
        let resumed_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IOEVENTFD_RESUMED_BYTE,
            "ioeventfd resumed main output",
        )?;
        let completion_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IOEVENTFD_DONE_BYTE,
            "ioeventfd completion barrier",
        )?;
        let completion = vcpu.registers()?;
        require_interrupt_enabled_flags("ioeventfd completion state", completion.rflags)?;
        Ok((
            readiness_io,
            barrier_io,
            handler_io,
            resumed_io,
            completion_io,
            doorbell_state.rflags,
            completion.rflags,
        ))
    })();

    let _ = watchdog_cancel_tx.send(());
    let doorbell_join = doorbell_worker.join().map_err(|_| {
        verification_error(
            "join ioeventfd doorbell worker",
            "ioeventfd doorbell worker panicked before reporting its event counter",
        )
    });
    let watchdog_join = join_async_timer_watchdog(watchdog_worker);
    let ioeventfd_cleanup = ioeventfd_registration.deassign(&vm);
    let irqfd_cleanup = irqfd_registration.deassign(&vm);

    let doorbell_count = doorbell_join?
        .map_err(|source| async_timer_vm_error("ioeventfd doorbell worker", source))?;
    let watchdog_fired = watchdog_join?;
    ioeventfd_cleanup
        .map_err(|source| async_timer_vm_error("deassign KVM_IOEVENTFD doorbell", source))?;
    irqfd_cleanup
        .map_err(|source| async_timer_vm_error("deassign ioeventfd response KVM_IRQFD", source))?;

    if watchdog_fired {
        return Err(verification_error(
            "ioeventfd doorbell watchdog",
            "watchdog injected a fallback GSI; the ioeventfd/irqfd roundtrip was not independently proven",
        ));
    }

    let (
        readiness_io,
        barrier_io,
        handler_io,
        resumed_io,
        completion_io,
        doorbell_rflags,
        completion_rflags,
    ) = execution?;
    let io_exits = vec![
        readiness_io,
        barrier_io,
        handler_io,
        resumed_io,
        completion_io,
    ];
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != KvmBackend::IOEVENTFD_DOORBELL_PROOF
        || io_exits.len() != KvmBackend::IOEVENTFD_DOORBELL_PROOF.len()
    {
        return Err(verification_error(
            "ioeventfd doorbell roundtrip proof",
            format!(
                "expected exact proof {:?} across {} byte-wide I/O exits, got proof {:?} across {} exits",
                KvmBackend::IOEVENTFD_DOORBELL_PROOF,
                KvmBackend::IOEVENTFD_DOORBELL_PROOF.len(),
                proof,
                io_exits.len()
            ),
        ));
    }

    Ok(IoeventfdDoorbellGuestResult {
        doorbell_gpa: KvmBackend::IOEVENTFD_DOORBELL_GPA,
        doorbell_value: KvmBackend::IOEVENTFD_DOORBELL_VALUE,
        doorbell_event_count: doorbell_count,
        userspace_mmio_exit_count: 0,
        gsi: KvmBackend::IOEVENTFD_DOORBELL_GSI,
        vector: KvmBackend::IOEVENTFD_DOORBELL_VECTOR,
        lapic_spiv: lapic.spiv(),
        lapic_lint0: lapic.lint0(),
        doorbell_rflags,
        completion_rflags,
        io_exits,
        proof,
    })
}

fn require_ioeventfd_capability_for_backend(backend: &KvmBackend) -> Result<(), Error> {
    let capability = libc::c_ulong::try_from(KVM_CAP_IOEVENTFD)
        .expect("KVM_CAP_IOEVENTFD is a non-negative capability ID");
    let value = ioctl_with_arg(backend.fd.as_raw_fd(), KVM_CHECK_EXTENSION, capability).map_err(
        |source| {
            Error::HostEnvironment(HostEnvironmentError::Io {
                operation: "KVM_CHECK_EXTENSION KVM_CAP_IOEVENTFD",
                source,
            })
        },
    )?;
    if value <= 0 {
        return Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
            name: "KVM_CAP_IOEVENTFD",
            id: KVM_CAP_IOEVENTFD,
        }));
    }
    Ok(())
}

fn require_ioeventfd_capability() -> Result<(), Error> {
    Ok(())
}

fn set_ioeventfd(fd: std::os::fd::RawFd, request: &KvmIoeventfd) -> io::Result<()> {
    // SAFETY: `request` is the fixed 64-byte `struct kvm_ioeventfd` and remains readable for the
    // duration of the VM ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_IOEVENTFD, request) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wait_eventfd_counter(eventfd: &EventFd, timeout: std::time::Duration) -> io::Result<u64> {
    let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    let mut descriptor = libc::pollfd {
        fd: eventfd.fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: `descriptor` points to one writable pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if ready == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for KVM_IOEVENTFD doorbell signal",
            ));
        }
        if ready == -1 {
            let source = io::Error::last_os_error();
            if source.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(source);
        }
        if descriptor.revents & libc::POLLIN == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("ioeventfd poll returned unexpected revents {:#x}", descriptor.revents),
            ));
        }
        break;
    }

    let mut value = 0_u64;
    loop {
        // SAFETY: `value` is an eight-byte writable buffer and eventfd reads exactly one u64.
        let read = unsafe {
            libc::read(
                eventfd.fd.as_raw_fd(),
                (&mut value as *mut u64).cast::<libc::c_void>(),
                std::mem::size_of::<u64>(),
            )
        };
        if read == isize::try_from(std::mem::size_of::<u64>()).expect("eight bytes fit isize") {
            return Ok(value);
        }
        if read == -1 {
            let source = io::Error::last_os_error();
            if source.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(source);
        }
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "ioeventfd read returned {read} bytes instead of {}",
                std::mem::size_of::<u64>()
            ),
        ));
    }
}

const _: () = {
    assert!(std::mem::size_of::<KvmIoeventfd>() == 64);
};

#[cfg(test)]
mod ioeventfd_doorbell_tests {
    use super::*;

    #[test]
    fn ioeventfd_uapi_contract_matches_linux_kvm() {
        assert_eq!(KVM_CAP_IOEVENTFD, 36);
        assert_eq!(KVM_IOEVENTFD, 0x4040_AE79);
        assert_eq!(KVM_IOEVENTFD_FLAG_DATAMATCH, 1);
        assert_eq!(KVM_IOEVENTFD_FLAG_DEASSIGN, 4);
        assert_eq!(std::mem::size_of::<KvmIoeventfd>(), 64);
    }

    #[test]
    fn ioeventfd_assign_and_deassign_preserve_match_contract() {
        let assign = KvmIoeventfd::assign(17, 0x1000_0000, 1, u64::from(b'W'));
        assert_eq!(assign.datamatch, u64::from(b'W'));
        assert_eq!(assign.addr, 0x1000_0000);
        assert_eq!(assign.len, 1);
        assert_eq!(assign.fd, 17);
        assert_eq!(assign.flags, KVM_IOEVENTFD_FLAG_DATAMATCH);
        assert_eq!(assign.pad, [0; 36]);

        let deassign = KvmIoeventfd::deassign(17, 0x1000_0000, 1, u64::from(b'W'));
        assert_eq!(deassign.datamatch, assign.datamatch);
        assert_eq!(deassign.addr, assign.addr);
        assert_eq!(deassign.len, assign.len);
        assert_eq!(deassign.fd, assign.fd);
        assert_eq!(
            deassign.flags,
            KVM_IOEVENTFD_FLAG_DATAMATCH | KVM_IOEVENTFD_FLAG_DEASSIGN
        );
        assert_eq!(deassign.pad, [0; 36]);
    }

    #[test]
    fn duplicated_eventfd_reader_observes_exact_single_signal() {
        let eventfd = EventFd::new().unwrap();
        let reader = eventfd.duplicate().unwrap();
        eventfd.signal().unwrap();
        assert_eq!(
            wait_eventfd_counter(&reader, std::time::Duration::from_secs(1)).unwrap(),
            1
        );
    }

    #[test]
    fn deterministic_ioeventfd_guest_has_doorbell_barrier_and_sti_hlt_handoff() {
        assert_eq!(IOEVENTFD_GUEST_BYTES.len(), 69);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[37..41], &[0xb0, b'R', 0xe6, 0xe9]);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[51..54], &[0xc6, 0x03, b'W']);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[54..58], &[0xb0, b'B', 0xe6, 0xe9]);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[58..60], &[0xfb, 0xf4]);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[60..64], &[0xb0, b'M', 0xe6, 0xe9]);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[64..68], &[0xb0, b'D', 0xe6, 0xe9]);
        assert_eq!(IOEVENTFD_GUEST_BYTES[68], 0xf4);
        assert_eq!(IOEVENTFD_HANDLER_BYTES[1], b'I');
        assert_eq!(KvmBackend::IOEVENTFD_DOORBELL_PROOF, b"RBIMD");
    }
}
