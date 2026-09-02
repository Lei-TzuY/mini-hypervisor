mod sys;

use crate::error::{Error, HostEnvironmentError, KvmCapabilityError};
use crate::vcpu::{Vcpu, VcpuId};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

const EXPECTED_KVM_API_VERSION: i32 = 12;
const KVM_CAP_USER_MEMORY: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub name: &'static str,
    pub id: i32,
    pub value: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCapabilities {
    pub api_version: i32,
    pub vcpu_mmap_size: i32,
    pub extensions: Vec<Capability>,
}

impl HostCapabilities {
    pub fn validate(&self) -> Result<(), Error> {
        if self.api_version != EXPECTED_KVM_API_VERSION {
            return Err(Error::KvmCapability(
                KvmCapabilityError::UnsupportedApiVersion {
                    expected: EXPECTED_KVM_API_VERSION,
                    actual: self.api_version,
                },
            ));
        }

        if self.vcpu_mmap_size <= 0 {
            return Err(Error::KvmCapability(
                KvmCapabilityError::InvalidVcpuMmapSize {
                    size: self.vcpu_mmap_size,
                },
            ));
        }

        let user_memory = self
            .extensions
            .iter()
            .find(|capability| capability.id == KVM_CAP_USER_MEMORY);
        match user_memory {
            Some(capability) if capability.value > 0 => Ok(()),
            _ => Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
                name: "KVM_CAP_USER_MEMORY",
                id: KVM_CAP_USER_MEMORY,
            })),
        }
    }
}

#[derive(Debug)]
pub struct KvmBackend {
    fd: File,
    capabilities: HostCapabilities,
}

impl KvmBackend {
    pub fn open() -> Result<Self, Error> {
        Self::open_path(Path::new("/dev/kvm"))
    }

    fn open_path(path: &Path) -> Result<Self, Error> {
        let fd = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(classify_open_error)?;

        let api_version = sys::ioctl_noarg(fd.as_raw_fd(), sys::KVM_GET_API_VERSION)
            .map_err(|source| host_io("KVM_GET_API_VERSION", source))?;
        let vcpu_mmap_size = sys::ioctl_noarg(fd.as_raw_fd(), sys::KVM_GET_VCPU_MMAP_SIZE)
            .map_err(|source| host_io("KVM_GET_VCPU_MMAP_SIZE", source))?;
        let capability_id = libc::c_ulong::try_from(KVM_CAP_USER_MEMORY)
            .expect("KVM capability identifiers are positive constants");
        let user_memory =
            sys::ioctl_with_arg(fd.as_raw_fd(), sys::KVM_CHECK_EXTENSION, capability_id)
                .map_err(|source| host_io("KVM_CHECK_EXTENSION(KVM_CAP_USER_MEMORY)", source))?;

        let capabilities = HostCapabilities {
            api_version,
            vcpu_mmap_size,
            extensions: vec![Capability {
                name: "KVM_CAP_USER_MEMORY",
                id: KVM_CAP_USER_MEMORY,
                value: user_memory,
            }],
        };
        capabilities.validate()?;

        Ok(Self { fd, capabilities })
    }

    #[must_use]
    pub fn capabilities(&self) -> &HostCapabilities {
        &self.capabilities
    }

    pub fn create_vm(&self) -> Result<Vm, Error> {
        let raw_fd =
            sys::ioctl_with_arg(self.fd.as_raw_fd(), sys::KVM_CREATE_VM, 0).map_err(|source| {
                Error::HostEnvironment(HostEnvironmentError::VmCreation { source })
            })?;
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        Ok(Vm {
            fd,
            vcpu_mmap_size: usize::try_from(self.capabilities.vcpu_mmap_size)
                .expect("validated positive i32 always fits usize"),
        })
    }
}

#[derive(Debug)]
pub struct Vm {
    fd: OwnedFd,
    vcpu_mmap_size: usize,
}

impl Vm {
    pub fn create_vcpu(&self, id: VcpuId) -> Result<Vcpu, Error> {
        let raw_fd = sys::ioctl_with_arg(
            self.fd.as_raw_fd(),
            sys::KVM_CREATE_VCPU,
            libc::c_ulong::from(id.get()),
        )
        .map_err(|source| {
            Error::HostEnvironment(HostEnvironmentError::VcpuCreation {
                id: id.get(),
                source,
            })
        })?;
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        Vcpu::from_kvm_fd(id, fd, self.vcpu_mmap_size)
    }
}

fn classify_open_error(source: io::Error) -> Error {
    match source.kind() {
        io::ErrorKind::NotFound => {
            Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { source })
        }
        io::ErrorKind::PermissionDenied => {
            Error::HostEnvironment(HostEnvironmentError::PermissionDenied { source })
        }
        _ => Error::HostEnvironment(HostEnvironmentError::Io {
            operation: "open /dev/kvm",
            source,
        }),
    }
}

pub(crate) fn host_io(operation: &'static str, source: io::Error) -> Error {
    Error::HostEnvironment(HostEnvironmentError::Io { operation, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_capabilities() -> HostCapabilities {
        HostCapabilities {
            api_version: EXPECTED_KVM_API_VERSION,
            vcpu_mmap_size: 4096,
            extensions: vec![Capability {
                name: "KVM_CAP_USER_MEMORY",
                id: KVM_CAP_USER_MEMORY,
                value: 1,
            }],
        }
    }

    #[test]
    fn accepts_expected_capabilities() {
        assert!(valid_capabilities().validate().is_ok());
    }

    #[test]
    fn rejects_wrong_api_version() {
        let mut capabilities = valid_capabilities();
        capabilities.api_version = 11;
        assert!(matches!(
            capabilities.validate(),
            Err(Error::KvmCapability(
                KvmCapabilityError::UnsupportedApiVersion { actual: 11, .. }
            ))
        ));
    }

    #[test]
    fn rejects_missing_required_extension() {
        let mut capabilities = valid_capabilities();
        capabilities.extensions.clear();
        assert!(matches!(
            capabilities.validate(),
            Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
                name: "KVM_CAP_USER_MEMORY",
                ..
            }))
        ));
    }

    #[test]
    fn rejects_disabled_required_extension() {
        let mut capabilities = valid_capabilities();
        capabilities.extensions[0].value = 0;
        assert!(matches!(
            capabilities.validate(),
            Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
                name: "KVM_CAP_USER_MEMORY",
                ..
            }))
        ));
    }

    #[test]
    fn rejects_non_positive_vcpu_mmap_size() {
        let mut capabilities = valid_capabilities();
        capabilities.vcpu_mmap_size = 0;
        assert!(matches!(
            capabilities.validate(),
            Err(Error::KvmCapability(
                KvmCapabilityError::InvalidVcpuMmapSize { size: 0 }
            ))
        ));
    }
}
