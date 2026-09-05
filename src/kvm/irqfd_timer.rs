const KVM_CAP_IRQFD: i32 = 32;
const KVM_IRQFD: libc::c_ulong = 0x4020_AE76;
const KVM_IRQFD_FLAG_DEASSIGN: u32 = 1 << 0;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmIrqfd {
    fd: u32,
    gsi: u32,
    flags: u32,
    resamplefd: u32,
    pad: [u8; 16],
}

impl KvmIrqfd {
    const fn assign(fd: u32, gsi: u32) -> Self {
        Self {
            fd,
            gsi,
            flags: 0,
            resamplefd: 0,
            pad: [0; 16],
        }
    }

    const fn deassign(fd: u32, gsi: u32) -> Self {
        Self {
            fd,
            gsi,
            flags: KVM_IRQFD_FLAG_DEASSIGN,
            resamplefd: 0,
            pad: [0; 16],
        }
    }
}

#[derive(Debug)]
struct EventFd {
    fd: OwnedFd,
}

impl EventFd {
    fn new() -> io::Result<Self> {
        // SAFETY: eventfd takes integer value/flags and returns a new owned descriptor on success.
        let raw_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if raw_fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful eventfd returned a new descriptor owned by this object.
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        Ok(Self { fd })
    }

    fn duplicate(&self) -> io::Result<Self> {
        // SAFETY: dup reads only the numeric descriptor and returns a new descriptor on success.
        let raw_fd = unsafe { libc::dup(self.fd.as_raw_fd()) };
        if raw_fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful dup returned a new descriptor owned by this object.
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        Ok(Self { fd })
    }

    fn raw_u32(&self) -> io::Result<u32> {
        u32::try_from(self.fd.as_raw_fd()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "eventfd descriptor unexpectedly did not fit the KVM u32 fd field",
            )
        })
    }

    fn signal(&self) -> io::Result<()> {
        let value = 1_u64.to_ne_bytes();
        loop {
            // SAFETY: `value` is an eight-byte readable buffer and eventfd accepts exactly one u64.
            let written = unsafe {
                libc::write(
                    self.fd.as_raw_fd(),
                    value.as_ptr().cast::<libc::c_void>(),
                    value.len(),
                )
            };
            if written == isize::try_from(value.len()).expect("eight bytes fit isize") {
                return Ok(());
            }
            if written == -1 {
                let source = io::Error::last_os_error();
                if source.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(source);
            }
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!(
                    "eventfd signal wrote {written} bytes instead of {}",
                    value.len()
                ),
            ));
        }
    }
}

#[derive(Debug)]
struct IrqfdTimerRegistration {
    eventfd: EventFd,
    gsi: u32,
}

impl IrqfdTimerRegistration {
    fn assign(vm: &Vm, gsi: u32) -> io::Result<Self> {
        let eventfd = EventFd::new()?;
        let request = KvmIrqfd::assign(eventfd.raw_u32()?, gsi);
        set_irqfd(vm.fd.as_raw_fd(), &request)?;
        Ok(Self { eventfd, gsi })
    }

    fn duplicate_signal_handle(&self) -> io::Result<EventFd> {
        self.eventfd.duplicate()
    }

    fn deassign(&self, vm: &Vm) -> io::Result<()> {
        let request = KvmIrqfd::deassign(self.eventfd.raw_u32()?, self.gsi);
        set_irqfd(vm.fd.as_raw_fd(), &request)
    }
}

impl KvmBackend {
    pub const IRQFD_TIMER_GSI: u32 = Self::ASYNC_TIMER_GSI;
    pub const IRQFD_TIMER_VECTOR: u8 = Self::ASYNC_TIMER_VECTOR;
    pub const IRQFD_TIMER_PROOF: &'static [u8; 5] = Self::ASYNC_TIMER_PROOF;

    pub fn run_irqfd_timer_interrupt_guest(
        config: VmConfig,
    ) -> Result<AsyncTimerInterruptGuestResult, Error> {
        run_timer_interrupt_guest(config, AsyncTimerDelivery::Irqfd)
    }
}

fn prepare_irqfd_async_timer_delivery(
    backend: &KvmBackend,
    vm: &Vm,
) -> Result<PreparedAsyncTimerDelivery, Error> {
    require_irqfd_capability(backend)?;

    // Establish a known inactive level before assigning the edge-triggered irqfd route. The
    // KVM_IRQFD ioctl itself then validates the eventfd/GSI registration synchronously before the
    // guest is permitted to enter the potentially blocking sti;hlt handoff.
    vm.set_gsi_level(KvmBackend::IRQFD_TIMER_GSI, false)?;
    let registration = IrqfdTimerRegistration::assign(vm, KvmBackend::IRQFD_TIMER_GSI)
        .map_err(|source| async_timer_vm_error("assign async timer KVM_IRQFD", source))?;
    let signal = registration
        .duplicate_signal_handle()
        .map_err(|source| async_timer_vm_error("duplicate async timer eventfd signal handle", source))?;

    let timer_worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(
            ASYNC_TIMER_DELAY_MILLIS,
        ));
        signal.signal()
    });

    Ok(PreparedAsyncTimerDelivery {
        timer_worker,
        irqfd_registration: Some(registration),
    })
}

fn require_irqfd_capability(backend: &KvmBackend) -> Result<(), Error> {
    let capability = libc::c_ulong::try_from(KVM_CAP_IRQFD)
        .expect("KVM_CAP_IRQFD is a non-negative capability ID");
    let value = ioctl_with_arg(backend.fd.as_raw_fd(), KVM_CHECK_EXTENSION, capability).map_err(
        |source| {
            Error::HostEnvironment(HostEnvironmentError::Io {
                operation: "KVM_CHECK_EXTENSION KVM_CAP_IRQFD",
                source,
            })
        },
    )?;
    if value <= 0 {
        return Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
            name: "KVM_CAP_IRQFD",
            id: KVM_CAP_IRQFD,
        }));
    }
    Ok(())
}

fn set_irqfd(fd: std::os::fd::RawFd, request: &KvmIrqfd) -> io::Result<()> {
    // SAFETY: `request` is the fixed 32-byte `struct kvm_irqfd` and remains readable for the
    // duration of the VM ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_IRQFD, request) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

const _: () = {
    assert!(std::mem::size_of::<KvmIrqfd>() == 32);
};

#[cfg(test)]
mod irqfd_timer_tests {
    use super::*;

    #[test]
    fn irqfd_uapi_contract_matches_linux_kvm() {
        assert_eq!(KVM_CAP_IRQFD, 32);
        assert_eq!(KVM_IRQFD, 0x4020_AE76);
        assert_eq!(KVM_IRQFD_FLAG_DEASSIGN, 1);
        assert_eq!(std::mem::size_of::<KvmIrqfd>(), 32);
    }

    #[test]
    fn irqfd_assign_and_deassign_requests_preserve_fd_and_gsi() {
        let assign = KvmIrqfd::assign(17, 3);
        assert_eq!(assign.fd, 17);
        assert_eq!(assign.gsi, 3);
        assert_eq!(assign.flags, 0);
        assert_eq!(assign.resamplefd, 0);
        assert_eq!(assign.pad, [0; 16]);

        let deassign = KvmIrqfd::deassign(17, 3);
        assert_eq!(deassign.fd, 17);
        assert_eq!(deassign.gsi, 3);
        assert_eq!(deassign.flags, KVM_IRQFD_FLAG_DEASSIGN);
        assert_eq!(deassign.resamplefd, 0);
        assert_eq!(deassign.pad, [0; 16]);
    }

    #[test]
    fn duplicated_eventfd_signal_handle_shares_one_counter() {
        let eventfd = EventFd::new().unwrap();
        let signal = eventfd.duplicate().unwrap();
        signal.signal().unwrap();

        let mut value = 0_u64;
        loop {
            // SAFETY: `value` is an eight-byte writable buffer and this test exclusively consumes
            // the single eventfd counter increment it just produced.
            let read = unsafe {
                libc::read(
                    eventfd.fd.as_raw_fd(),
                    (&mut value as *mut u64).cast::<libc::c_void>(),
                    std::mem::size_of::<u64>(),
                )
            };
            if read == isize::try_from(std::mem::size_of::<u64>()).unwrap() {
                break;
            }
            if read == -1 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            panic!("eventfd test read failed: {}", io::Error::last_os_error());
        }
        assert_eq!(value, 1);
    }
}
