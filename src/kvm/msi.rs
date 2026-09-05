const KVM_CAP_SIGNAL_MSI: i32 = 77;
const KVM_SIGNAL_MSI: libc::c_ulong = 0x4020_AEA5;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmMsi {
    address_lo: u32,
    address_hi: u32,
    data: u32,
    flags: u32,
    devid: u32,
    pad: [u8; 12],
}

impl KvmMsi {
    const fn new(address: u64, data: u32) -> Self {
        Self {
            address_lo: address as u32,
            address_hi: (address >> 32) as u32,
            data,
            flags: 0,
            devid: 0,
            pad: [0; 12],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KvmMsiMessage {
    address: u64,
    data: u32,
}

impl KvmMsiMessage {
    #[must_use]
    pub(crate) const fn new(address: u64, data: u32) -> Self {
        Self { address, data }
    }

    #[must_use]
    pub(crate) const fn address(self) -> u64 {
        self.address
    }

    #[must_use]
    pub(crate) const fn data(self) -> u32 {
        self.data
    }
}

impl crate::kvm::KvmBackend {
    pub(crate) fn require_signal_msi_capability(&self) -> Result<(), crate::error::Error> {
        let capability = libc::c_ulong::try_from(KVM_CAP_SIGNAL_MSI)
            .expect("KVM_CAP_SIGNAL_MSI is a non-negative capability ID");
        let value = ioctl_with_arg(
            std::os::fd::AsRawFd::as_raw_fd(&self.fd),
            KVM_CHECK_EXTENSION,
            capability,
        )
        .map_err(|source| {
            crate::error::Error::HostEnvironment(crate::error::HostEnvironmentError::Io {
                operation: "KVM_CHECK_EXTENSION KVM_CAP_SIGNAL_MSI",
                source,
            })
        })?;
        if value <= 0 {
            return Err(crate::error::Error::KvmCapability(
                crate::error::KvmCapabilityError::MissingExtension {
                    name: "KVM_CAP_SIGNAL_MSI",
                    id: KVM_CAP_SIGNAL_MSI,
                },
            ));
        }
        Ok(())
    }
}

impl crate::kvm::Vm {
    pub(crate) fn signal_msi(
        &self,
        message: KvmMsiMessage,
    ) -> Result<u32, crate::error::Error> {
        let request = KvmMsi::new(message.address(), message.data());
        // SAFETY: `request` is the exact fixed-size Linux `struct kvm_msi` payload and remains
        // readable for the duration of the VM ioctl.
        let result = unsafe {
            libc::ioctl(
                std::os::fd::AsRawFd::as_raw_fd(&self.fd),
                KVM_SIGNAL_MSI,
                &request,
            )
        };
        if result == -1 {
            return Err(crate::error::Error::HostEnvironment(
                crate::error::HostEnvironmentError::VmOperation {
                    operation: "KVM_SIGNAL_MSI",
                    source: std::io::Error::last_os_error(),
                },
            ));
        }
        if result == 0 {
            return Err(crate::error::Error::HostEnvironment(
                crate::error::HostEnvironmentError::VmOperation {
                    operation: "KVM_SIGNAL_MSI",
                    source: std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "KVM_SIGNAL_MSI reported a coalesced or blocked message instead of delivery",
                    ),
                },
            ));
        }
        u32::try_from(result).map_err(|_| {
            crate::error::Error::HostEnvironment(crate::error::HostEnvironmentError::VmOperation {
                operation: "KVM_SIGNAL_MSI",
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("KVM_SIGNAL_MSI returned invalid delivery count {result}"),
                ),
            })
        })
    }
}

const _: () = {
    assert!(std::mem::size_of::<KvmMsi>() == 32);
};

#[cfg(test)]
mod msi_tests {
    use super::*;

    #[test]
    fn signal_msi_uapi_matches_linux_kvm() {
        assert_eq!(KVM_CAP_SIGNAL_MSI, 77);
        assert_eq!(KVM_SIGNAL_MSI, 0x4020_AEA5);
        assert_eq!(std::mem::size_of::<KvmMsi>(), 32);
    }

    #[test]
    fn msi_request_preserves_address_data_and_zeroes_optional_fields() {
        let request = KvmMsi::new(0x0000_0000_fee0_0000, 0x50);
        assert_eq!(request.address_lo, 0xfee0_0000);
        assert_eq!(request.address_hi, 0);
        assert_eq!(request.data, 0x50);
        assert_eq!(request.flags, 0);
        assert_eq!(request.devid, 0);
        assert_eq!(request.pad, [0; 12]);
    }
}
