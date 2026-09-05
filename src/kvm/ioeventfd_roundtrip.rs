const KVM_CAP_IOEVENTFD: i32 = 36;
const KVM_IOEVENTFD: libc::c_ulong = 0x4040_AE79;
const KVM_IOEVENTFD_FLAG_DATAMATCH: u32 = 1 << 0;
const KVM_IOEVENTFD_FLAG_DEASSIGN: u32 = 1 << 2;
const IOEVENTFD_DOORBELL_VALUE: u8 = 0x5a;
const IOEVENTFD_READY_BYTE: u8 = b'R';
const IOEVENTFD_ARMED_BYTE: u8 = b'A';
const IOEVENTFD_HANDLER_BYTE: u8 = b'T';
const IOEVENTFD_WOKE_BYTE: u8 = b'W';
const IOEVENTFD_DONE_BYTE: u8 = b'D';
const IOEVENTFD_WAIT_TIMEOUT_MILLIS: i32 = 5_000;

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
    0xb0, IOEVENTFD_READY_BYTE, 0xe6, 0xe9, // host installs accelerated transports after R
    0x48, 0xbb, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, // movabs $0x500000, %rbx
    0xc6, 0x03, IOEVENTFD_DOORBELL_VALUE, // one-byte MMIO doorbell write -> KVM_IOEVENTFD
    0xb0, IOEVENTFD_ARMED_BYTE, 0xe6, 0xe9, // doorbell completion barrier, IF still clear
    0xfb, // sti
    0xf4, // hlt -- pending irqfd edge safely completes STI/HLT handoff
    0xb0, IOEVENTFD_WOKE_BYTE, 0xe6, 0xe9, // resumed mainline after handler
    0xb0, IOEVENTFD_DONE_BYTE, 0xe6, 0xe9, // terminal userspace barrier
    0xf4, // safety fallback; host deliberately stops at D
];

const IOEVENTFD_HANDLER_BYTES: [u8; 10] = [
    0xb0, IOEVENTFD_HANDLER_BYTE, 0xe6, 0xe9, // accelerated round-trip handler proof
    0xb0, 0x20, 0xe6, 0x20, // non-specific EOI to master PIC
    0x48, 0xcf, // iretq
];

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmIoEventFd {
    datamatch: u64,
    addr: u64,
    len: u32,
    fd: i32,
    flags: u32,
    pad: [u8; 36],
}

impl KvmIoEventFd {
    const fn assign_mmio_datamatch(fd: i32, addr: u64, value: u8) -> Self {
        Self {
            datamatch: value as u64,
            addr,
            len: 1,
            fd,
            flags: KVM_IOEVENTFD_FLAG_DATAMATCH,
            pad: [0; 36],
        }
    }

    const fn deassign_mmio_datamatch(fd: i32, addr: u64, value: u8) -> Self {
        Self {
            datamatch: value as u64,
            addr,
            len: 1,
            fd,
            flags: KVM_IOEVENTFD_FLAG_DATAMATCH | KVM_IOEVENTFD_FLAG_DEASSIGN,
            pad: [0; 36],
        }
    }
}

#[derive(Debug)]
struct PreparedIoEventFdDoorbell {
    eventfd: EventFd,
    reader: EventFd,
    address: u64,
    value: u8,
}

impl PreparedIoEventFdDoorbell {
    fn new(address: u64, value: u8) -> io::Result<Self> {
        let eventfd = EventFd::new()?;
        let reader = eventfd.duplicate()?;
        Ok(Self {
            eventfd,
            reader,
            address,
            value,
        })
    }

    fn assign(self, vm: &Vm) -> io::Result<(IoEventFdDoorbellRegistration, EventFd)> {
        let request = KvmIoEventFd::assign_mmio_datamatch(
            self.eventfd.fd.as_raw_fd(),
            self.address,
            self.value,
        );
        set_ioeventfd(vm.fd.as_raw_fd(), &request)?;
        Ok((
            IoEventFdDoorbellRegistration {
                eventfd: self.eventfd,
                address: self.address,
                value: self.value,
            },
            self.reader,
        ))
    }
}

#[derive(Debug)]
struct IoEventFdDoorbellRegistration {
    eventfd: EventFd,
    address: u64,
    value: u8,
}

impl IoEventFdDoorbellRegistration {
    fn deassign(&self, vm: &Vm) -> io::Result<()> {
        let request = KvmIoEventFd::deassign_mmio_datamatch(
            self.eventfd.fd.as_raw_fd(),
            self.address,
            self.value,
        );
        set_ioeventfd(vm.fd.as_raw_fd(), &request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoEventFdIrqfdRoundtripResult {
    doorbell_gpa: u64,
    doorbell_value: u8,
    doorbell_events: u64,
    gsi: u32,
    vector: u8,
    lapic_spiv: u32,
    lapic_lint0: u32,
    armed_rflags: u64,
    completion_rflags: u64,
    io_exits: Vec<PortIoExit>,
    proof: Vec<u8>,
}

impl IoEventFdIrqfdRoundtripResult {
    #[must_use]
    pub const fn doorbell_gpa(&self) -> u64 {
        self.doorbell_gpa
    }

    #[must_use]
    pub const fn doorbell_value(&self) -> u8 {
        self.doorbell_value
    }

    #[must_use]
    pub const fn doorbell_events(&self) -> u64 {
        self.doorbell_events
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
    pub const IOEVENTFD_ROUNDTRIP_DOORBELL_GPA: u64 =
        crate::mmio::long_mode::LONG_MODE_MMIO_DEVICE_GPA;
    pub const IOEVENTFD_ROUNDTRIP_DOORBELL_VALUE: u8 = IOEVENTFD_DOORBELL_VALUE;
    pub const IOEVENTFD_ROUNDTRIP_GSI: u32 = Self::IRQFD_TIMER_GSI;
    pub const IOEVENTFD_ROUNDTRIP_VECTOR: u8 = Self::IRQFD_TIMER_VECTOR;
    pub const IOEVENTFD_ROUNDTRIP_PROOF: &'static [u8; 5] = b"RATWD";

    pub fn run_ioeventfd_irqfd_roundtrip_guest(
        config: VmConfig,
    ) -> Result<IoEventFdIrqfdRoundtripResult, Error> {
        run_ioeventfd_irqfd_roundtrip(config)
    }
}

fn run_ioeventfd_irqfd_roundtrip(
    config: VmConfig,
) -> Result<IoEventFdIrqfdRoundtripResult, Error> {
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
    require_irqfd_capability(&backend)?;
    require_ioeventfd_capability(&backend)?;

    let mut vm = backend.create_vm_with_irqchip()?;
    let mut memory = GuestMemory::new(GuestPhysAddr::new(0), LONG_MODE_IDENTITY_MAP_SIZE)?;
    let layout = crate::mmio::interrupt::LongModeMmioInterruptLayout::new(
        memory.region(),
        guest.entry(),
        crate::mmio::long_mode::LONG_MODE_MMIO_STACK_POINTER,
        KvmBackend::IOEVENTFD_ROUNDTRIP_VECTOR,
        handler.entry(),
    )
    .expect("fixed ioeventfd/irqfd round-trip layout remains valid");
    layout.install_tables(&mut memory)?;
    guest.load(&mut memory)?;
    handler.load(&mut memory)?;
    vm.register_guest_memory(memory)?;

    debug_assert_eq!(config.vcpu_count(), 1);
    let mut vcpu = vm.create_vcpu(VcpuId::BOOT)?;
    vcpu.initialize_long_mode_interrupts(layout.interrupt_layout())?;
    let lapic = vcpu.configure_legacy_pic_extint()?;
    let mut port_io = PortIoBus::with_debug_port();

    let readiness_io = run_expected_debug_output(
        &mut vcpu,
        &mut port_io,
        IOEVENTFD_READY_BYTE,
        "ioeventfd round-trip readiness output",
    )?;
    let readiness = vcpu.registers()?;
    require_interrupt_disabled_flags("ioeventfd round-trip readiness state", readiness.rflags)?;

    // Finish every userspace fd allocation/duplication before either accelerated registration is
    // installed in KVM. The watchdog VM-fd handle is also preflighted before kernel registration.
    let doorbell = PreparedIoEventFdDoorbell::new(
        KvmBackend::IOEVENTFD_ROUNDTRIP_DOORBELL_GPA,
        KvmBackend::IOEVENTFD_ROUNDTRIP_DOORBELL_VALUE,
    )
    .map_err(|source| roundtrip_vm_error("prepare ioeventfd doorbell eventfd", source))?;
    let watchdog_irq = vm
        .duplicate_irq_line_handle()
        .map_err(|source| roundtrip_vm_error("duplicate round-trip watchdog IRQ-line handle", source))?;
    watchdog_irq
        .set_gsi_level(KvmBackend::IOEVENTFD_ROUNDTRIP_GSI, false)
        .map_err(|source| roundtrip_vm_error("preflight round-trip watchdog IRQ-line handle", source))?;
    vm.set_gsi_level(KvmBackend::IOEVENTFD_ROUNDTRIP_GSI, false)?;

    // KVM_IRQFD prepares both eventfd descriptors before assignment. If the later ioeventfd
    // assignment fails, explicitly remove irqfd before returning so no kernel registration leaks.
    let (irqfd_registration, irq_signal) = IrqfdTimerRegistration::assign_with_signal(
        &vm,
        KvmBackend::IOEVENTFD_ROUNDTRIP_GSI,
    )
    .map_err(|source| roundtrip_vm_error("assign round-trip KVM_IRQFD", source))?;
    let (doorbell_registration, doorbell_reader) = match doorbell.assign(&vm) {
        Ok(assigned) => assigned,
        Err(source) => {
            irqfd_registration.deassign(&vm).map_err(|cleanup| {
                roundtrip_vm_error("cleanup irqfd after ioeventfd assign failure", cleanup)
            })?;
            return Err(roundtrip_vm_error("assign round-trip KVM_IOEVENTFD", source));
        }
    };

    let bridge_worker = std::thread::spawn(move || -> io::Result<u64> {
        let count = wait_eventfd_value(&doorbell_reader, IOEVENTFD_WAIT_TIMEOUT_MILLIS)?;
        if count != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ioeventfd doorbell counter was {count}; expected exactly 1"),
            ));
        }
        irq_signal.signal()?;
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
                watchdog_irq.pulse_gsi_edge(KvmBackend::IOEVENTFD_ROUNDTRIP_GSI)?;
                Ok(true)
            }
        }
    });

    let execution = (|| -> Result<_, Error> {
        // Re-entering from R executes the registered MMIO doorbell write entirely in KVM. If
        // KVM_IOEVENTFD does not consume it, run_expected_debug_output fails on KVM_EXIT_MMIO
        // instead of silently accepting a userspace-emulated fallback.
        let armed_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IOEVENTFD_ARMED_BYTE,
            "ioeventfd round-trip armed barrier",
        )?;
        let armed = vcpu.registers()?;
        require_interrupt_disabled_flags("ioeventfd round-trip armed state", armed.rflags)?;

        let handler_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IOEVENTFD_HANDLER_BYTE,
            "ioeventfd round-trip handler output",
        )?;
        let woke_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IOEVENTFD_WOKE_BYTE,
            "ioeventfd round-trip resumed-main output",
        )?;
        let completion_io = run_expected_debug_output(
            &mut vcpu,
            &mut port_io,
            IOEVENTFD_DONE_BYTE,
            "ioeventfd round-trip completion barrier",
        )?;
        let completion = vcpu.registers()?;
        require_interrupt_enabled_flags("ioeventfd round-trip completion state", completion.rflags)?;
        Ok((armed_io, armed.rflags, handler_io, woke_io, completion_io, completion.rflags))
    })();

    let _ = watchdog_cancel_tx.send(());
    let bridge_join = bridge_worker.join().map_err(|_| {
        verification_error(
            "join ioeventfd/irqfd bridge worker",
            "ioeventfd bridge worker panicked before reporting the doorbell count",
        )
    });
    let watchdog_join = join_async_timer_watchdog(watchdog_worker);

    // Remove both kernel-assisted registrations before accepting worker or guest proof results.
    let ioeventfd_cleanup = doorbell_registration.deassign(&vm);
    let irqfd_cleanup = irqfd_registration.deassign(&vm);
    ioeventfd_cleanup
        .map_err(|source| roundtrip_vm_error("deassign round-trip KVM_IOEVENTFD", source))?;
    irqfd_cleanup.map_err(|source| roundtrip_vm_error("deassign round-trip KVM_IRQFD", source))?;

    let bridge_result = bridge_join?;
    let doorbell_events = bridge_result
        .map_err(|source| roundtrip_vm_error("ioeventfd/irqfd bridge worker", source))?;
    let watchdog_fired = watchdog_join?;
    if watchdog_fired {
        return Err(verification_error(
            "ioeventfd/irqfd round-trip watchdog",
            "watchdog injected a fallback GSI; accelerated round-trip was not independently proven",
        ));
    }

    let (armed_io, armed_rflags, handler_io, woke_io, completion_io, completion_rflags) = execution?;
    let io_exits = vec![readiness_io, armed_io, handler_io, woke_io, completion_io];
    let proof = port_io.debug_output().unwrap_or(&[]).to_vec();
    if proof.as_slice() != KvmBackend::IOEVENTFD_ROUNDTRIP_PROOF
        || io_exits.len() != KvmBackend::IOEVENTFD_ROUNDTRIP_PROOF.len()
    {
        return Err(verification_error(
            "ioeventfd/irqfd round-trip proof",
            format!(
                "expected exact proof {:?} across {} byte-wide I/O exits, got proof {:?} across {} exits",
                KvmBackend::IOEVENTFD_ROUNDTRIP_PROOF,
                KvmBackend::IOEVENTFD_ROUNDTRIP_PROOF.len(),
                proof,
                io_exits.len()
            ),
        ));
    }

    Ok(IoEventFdIrqfdRoundtripResult {
        doorbell_gpa: KvmBackend::IOEVENTFD_ROUNDTRIP_DOORBELL_GPA,
        doorbell_value: KvmBackend::IOEVENTFD_ROUNDTRIP_DOORBELL_VALUE,
        doorbell_events,
        gsi: KvmBackend::IOEVENTFD_ROUNDTRIP_GSI,
        vector: KvmBackend::IOEVENTFD_ROUNDTRIP_VECTOR,
        lapic_spiv: lapic.spiv(),
        lapic_lint0: lapic.lint0(),
        armed_rflags,
        completion_rflags,
        io_exits,
        proof,
    })
}

fn require_ioeventfd_capability(backend: &KvmBackend) -> Result<(), Error> {
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

fn set_ioeventfd(fd: std::os::fd::RawFd, request: &KvmIoEventFd) -> io::Result<()> {
    // SAFETY: `request` is the fixed 64-byte Linux `struct kvm_ioeventfd` and remains readable for
    // the duration of the VM ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_IOEVENTFD, request) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wait_eventfd_value(eventfd: &EventFd, timeout_millis: i32) -> io::Result<u64> {
    let mut pollfd = libc::pollfd {
        fd: eventfd.fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: `pollfd` points to one initialized poll descriptor for the duration of the call.
        let result = unsafe { libc::poll(&mut pollfd, 1, timeout_millis) };
        if result == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for KVM_IOEVENTFD doorbell event",
            ));
        }
        if result == -1 {
            let source = io::Error::last_os_error();
            if source.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(source);
        }
        if pollfd.revents & libc::POLLIN == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("unexpected poll revents for ioeventfd: {:#x}", pollfd.revents),
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
            format!("ioeventfd read returned {read} bytes instead of 8"),
        ));
    }
}

fn roundtrip_vm_error(operation: &'static str, source: io::Error) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VmOperation { operation, source })
}

const _: () = {
    assert!(std::mem::size_of::<KvmIoEventFd>() == 64);
};

#[cfg(test)]
mod ioeventfd_roundtrip_tests {
    use super::*;

    #[test]
    fn ioeventfd_uapi_contract_matches_linux_kvm() {
        assert_eq!(KVM_CAP_IOEVENTFD, 36);
        assert_eq!(KVM_IOEVENTFD, 0x4040_AE79);
        assert_eq!(KVM_IOEVENTFD_FLAG_DATAMATCH, 1);
        assert_eq!(KVM_IOEVENTFD_FLAG_DEASSIGN, 4);
        assert_eq!(std::mem::size_of::<KvmIoEventFd>(), 64);
    }

    #[test]
    fn ioeventfd_assign_and_deassign_preserve_exact_mmio_datamatch() {
        let assign = KvmIoEventFd::assign_mmio_datamatch(17, 0x1000_0000, 0x5a);
        assert_eq!(assign.datamatch, 0x5a);
        assert_eq!(assign.addr, 0x1000_0000);
        assert_eq!(assign.len, 1);
        assert_eq!(assign.fd, 17);
        assert_eq!(assign.flags, KVM_IOEVENTFD_FLAG_DATAMATCH);
        assert_eq!(assign.pad, [0; 36]);

        let deassign = KvmIoEventFd::deassign_mmio_datamatch(17, 0x1000_0000, 0x5a);
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
    fn deterministic_roundtrip_guest_places_doorbell_before_if_enable_handoff() {
        assert_eq!(IOEVENTFD_GUEST_BYTES.len(), 69);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[37..41], &[0xb0, b'R', 0xe6, 0xe9]);
        assert_eq!(
            &IOEVENTFD_GUEST_BYTES[41..51],
            &[0x48, 0xbb, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(&IOEVENTFD_GUEST_BYTES[51..54], &[0xc6, 0x03, 0x5a]);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[54..58], &[0xb0, b'A', 0xe6, 0xe9]);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[58..60], &[0xfb, 0xf4]);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[60..64], &[0xb0, b'W', 0xe6, 0xe9]);
        assert_eq!(&IOEVENTFD_GUEST_BYTES[64..68], &[0xb0, b'D', 0xe6, 0xe9]);
        assert_eq!(IOEVENTFD_GUEST_BYTES[68], 0xf4);
        assert_eq!(IOEVENTFD_HANDLER_BYTES[1], b'T');
    }
}
