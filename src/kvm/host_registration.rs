#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostRegistrationSpec {
    doorbell_address: u64,
    doorbell_length: u32,
    doorbell_datamatch: u64,
    gsi: u32,
}

impl HostRegistrationSpec {
    pub(crate) fn new(
        doorbell_address: u64,
        doorbell_length: u32,
        doorbell_datamatch: u64,
        gsi: u32,
    ) -> Result<Self, Error> {
        if !matches!(doorbell_length, 1 | 2 | 4 | 8) {
            return Err(host_registration_error(
                "validate host registration descriptor",
                format!(
                    "ioeventfd doorbell length {doorbell_length} is unsupported; expected 1, 2, 4 or 8"
                ),
            ));
        }
        let significant_bits = doorbell_length * 8;
        if significant_bits < 64 && doorbell_datamatch >= (1_u64 << significant_bits) {
            return Err(host_registration_error(
                "validate host registration descriptor",
                format!(
                    "ioeventfd datamatch {doorbell_datamatch:#x} does not fit {doorbell_length} bytes"
                ),
            ));
        }
        doorbell_address
            .checked_add(u64::from(doorbell_length))
            .ok_or_else(|| {
                host_registration_error(
                    "validate host registration descriptor",
                    format!(
                        "ioeventfd doorbell range {doorbell_address:#x}+{doorbell_length:#x} overflows"
                    ),
                )
            })?;
        Ok(Self {
            doorbell_address,
            doorbell_length,
            doorbell_datamatch,
            gsi,
        })
    }

    #[must_use]
    pub(crate) const fn doorbell_address(self) -> u64 {
        self.doorbell_address
    }

    #[must_use]
    pub(crate) const fn doorbell_length(self) -> u32 {
        self.doorbell_length
    }

    #[must_use]
    pub(crate) const fn doorbell_datamatch(self) -> u64 {
        self.doorbell_datamatch
    }

    #[must_use]
    pub(crate) const fn gsi(self) -> u32 {
        self.gsi
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostRegistrationCheckpoint {
    spec: HostRegistrationSpec,
}

impl HostRegistrationCheckpoint {
    #[must_use]
    pub(crate) const fn capture(spec: HostRegistrationSpec) -> Self {
        Self { spec }
    }

    pub(crate) fn reconstruct(
        self,
        backend: &KvmBackend,
        vm: &Vm,
    ) -> Result<ReconstructedHostRegistrations, Error> {
        ReconstructedHostRegistrations::reconstruct(backend, vm, self.spec)
    }
}

#[derive(Debug)]
pub(crate) struct ReconstructedHostRegistrations {
    spec: HostRegistrationSpec,
    doorbell_eventfd: EventFd,
    doorbell_reader: EventFd,
    irqfd_registration: IrqfdTimerRegistration,
    irq_signal: EventFd,
}

impl ReconstructedHostRegistrations {
    pub(crate) fn reconstruct(
        backend: &KvmBackend,
        vm: &Vm,
        spec: HostRegistrationSpec,
    ) -> Result<Self, Error> {
        require_irqfd_capability(backend)?;
        require_ioeventfd_capability(backend)?;

        vm.set_gsi_level(spec.gsi(), false)?;

        let doorbell_eventfd = EventFd::new().map_err(|source| {
            host_registration_error_with_source("create reconstructed ioeventfd eventfd", source)
        })?;
        let doorbell_reader = doorbell_eventfd.duplicate().map_err(|source| {
            host_registration_error_with_source("duplicate reconstructed ioeventfd reader", source)
        })?;

        let (irqfd_registration, irq_signal) =
            IrqfdTimerRegistration::assign_with_signal(vm, spec.gsi()).map_err(|source| {
                host_registration_error_with_source("assign reconstructed KVM_IRQFD", source)
            })?;

        let request = KvmIoEventFd {
            datamatch: spec.doorbell_datamatch(),
            addr: spec.doorbell_address(),
            len: spec.doorbell_length(),
            fd: doorbell_eventfd.fd.as_raw_fd(),
            flags: KVM_IOEVENTFD_FLAG_DATAMATCH,
            pad: [0; 36],
        };
        if let Err(source) = set_ioeventfd(vm.fd.as_raw_fd(), &request) {
            let cleanup = irqfd_registration.deassign(vm);
            if let Err(cleanup) = cleanup {
                return Err(host_registration_error_with_source(
                    "cleanup reconstructed irqfd after ioeventfd assign failure",
                    cleanup,
                ));
            }
            return Err(host_registration_error_with_source(
                "assign reconstructed KVM_IOEVENTFD",
                source,
            ));
        }

        Ok(Self {
            spec,
            doorbell_eventfd,
            doorbell_reader,
            irqfd_registration,
            irq_signal,
        })
    }

    pub(crate) fn wait_doorbell(&self, timeout_millis: i32) -> Result<u64, Error> {
        wait_eventfd_value(&self.doorbell_reader, timeout_millis).map_err(|source| {
            host_registration_error_with_source("wait for reconstructed ioeventfd doorbell", source)
        })
    }

    pub(crate) fn doorbell_pending(&self) -> Result<bool, Error> {
        eventfd_pending(&self.doorbell_reader).map_err(|source| {
            host_registration_error_with_source(
                "probe reconstructed ioeventfd checkpoint quiescence",
                source,
            )
        })
    }

    pub(crate) fn signal_irq(&self) -> Result<(), Error> {
        self.irq_signal.signal().map_err(|source| {
            host_registration_error_with_source("signal reconstructed irqfd eventfd", source)
        })
    }

    pub(crate) fn deassign(self, vm: &Vm) -> Result<(), Error> {
        let ioeventfd_request = KvmIoEventFd {
            datamatch: self.spec.doorbell_datamatch(),
            addr: self.spec.doorbell_address(),
            len: self.spec.doorbell_length(),
            fd: self.doorbell_eventfd.fd.as_raw_fd(),
            flags: KVM_IOEVENTFD_FLAG_DATAMATCH | KVM_IOEVENTFD_FLAG_DEASSIGN,
            pad: [0; 36],
        };
        let ioeventfd_result = set_ioeventfd(vm.fd.as_raw_fd(), &ioeventfd_request);
        let irqfd_result = self.irqfd_registration.deassign(vm);

        match (ioeventfd_result, irqfd_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(source), _) => Err(host_registration_error_with_source(
                "deassign reconstructed KVM_IOEVENTFD",
                source,
            )),
            (Ok(()), Err(source)) => Err(host_registration_error_with_source(
                "deassign reconstructed KVM_IRQFD",
                source,
            )),
        }
    }
}

fn eventfd_pending(eventfd: &EventFd) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd: eventfd.fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: poll receives one valid pollfd and does not retain the pointer after returning.
        let ready = unsafe { libc::poll(&mut pollfd, 1, 0) };
        if ready == 0 {
            return Ok(false);
        }
        if ready == -1 {
            let source = io::Error::last_os_error();
            if source.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(source);
        }
        let terminal = libc::POLLERR | libc::POLLHUP | libc::POLLNVAL;
        if pollfd.revents & terminal != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "eventfd checkpoint-quiescence poll returned terminal revents {:#x}",
                    pollfd.revents
                ),
            ));
        }
        return Ok(pollfd.revents & libc::POLLIN != 0);
    }
}

fn host_registration_error(operation: &'static str, detail: impl Into<String>) -> Error {
    host_registration_error_with_source(
        operation,
        std::io::Error::new(std::io::ErrorKind::InvalidInput, detail.into()),
    )
}

fn host_registration_error_with_source(operation: &'static str, source: std::io::Error) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VmOperation { operation, source })
}

#[cfg(test)]
mod host_registration_tests {
    use super::*;

    #[test]
    fn checkpoint_quiescence_probe_is_non_consuming() {
        let eventfd = EventFd::new().unwrap();
        assert!(!eventfd_pending(&eventfd).unwrap());

        eventfd.signal().unwrap();
        assert!(eventfd_pending(&eventfd).unwrap());
        assert!(eventfd_pending(&eventfd).unwrap());
        assert_eq!(wait_eventfd_value(&eventfd, 0).unwrap(), 1);
        assert!(!eventfd_pending(&eventfd).unwrap());
    }

    #[test]
    fn descriptor_owns_semantics_without_a_raw_fd() {
        let spec = HostRegistrationSpec::new(0x1000_0100, 2, 0, 0).unwrap();
        assert_eq!(spec.doorbell_address(), 0x1000_0100);
        assert_eq!(spec.doorbell_length(), 2);
        assert_eq!(spec.doorbell_datamatch(), 0);
        assert_eq!(spec.gsi(), 0);
        assert_eq!(std::mem::size_of::<HostRegistrationSpec>(), 24);
        let checkpoint = HostRegistrationCheckpoint::capture(spec);
        assert_eq!(checkpoint.spec, spec);
        assert_eq!(std::mem::size_of::<HostRegistrationCheckpoint>(), 24);
    }

    #[test]
    fn descriptor_rejects_invalid_lengths_overflow_and_datamatch_width() {
        assert!(HostRegistrationSpec::new(0x1000, 3, 0, 0).is_err());
        assert!(HostRegistrationSpec::new(u64::MAX, 2, 0, 0).is_err());
        assert!(HostRegistrationSpec::new(0x1000, 1, 0x100, 0).is_err());
        assert!(HostRegistrationSpec::new(0x1000, 2, 0x1_0000, 0).is_err());
        assert!(HostRegistrationSpec::new(0x1000, 8, u64::MAX, 0).is_ok());
    }

    #[test]
    fn reconstructed_ioeventfd_requests_preserve_descriptor_semantics() {
        let spec = HostRegistrationSpec::new(0x1000_0100, 2, 0, 4).unwrap();
        let assign = KvmIoEventFd {
            datamatch: spec.doorbell_datamatch(),
            addr: spec.doorbell_address(),
            len: spec.doorbell_length(),
            fd: 17,
            flags: KVM_IOEVENTFD_FLAG_DATAMATCH,
            pad: [0; 36],
        };
        assert_eq!(assign.addr, 0x1000_0100);
        assert_eq!(assign.len, 2);
        assert_eq!(assign.datamatch, 0);
        assert_eq!(assign.fd, 17);
        assert_eq!(assign.flags, KVM_IOEVENTFD_FLAG_DATAMATCH);
    }
}
