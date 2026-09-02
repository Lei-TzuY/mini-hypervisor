use crate::error::{Error, HostEnvironmentError};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr::NonNull;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VcpuId(u16);

impl VcpuId {
    pub const BOOT: Self = Self(0);

    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug)]
pub struct Vcpu {
    id: VcpuId,
    _fd: OwnedFd,
    _run: KvmRunMapping,
}

impl Vcpu {
    pub(crate) fn from_kvm_fd(
        id: VcpuId,
        fd: OwnedFd,
        run_mmap_size: usize,
    ) -> Result<Self, Error> {
        let run = KvmRunMapping::map(&fd, run_mmap_size).map_err(|source| {
            Error::HostEnvironment(HostEnvironmentError::VcpuRunMapping {
                id: id.get(),
                source,
            })
        })?;
        Ok(Self {
            id,
            _fd: fd,
            _run: run,
        })
    }

    #[must_use]
    pub const fn id(&self) -> VcpuId {
        self.id
    }
}

#[derive(Debug)]
struct KvmRunMapping {
    ptr: NonNull<libc::c_void>,
    len: usize,
}

impl KvmRunMapping {
    fn map(fd: &OwnedFd, len: usize) -> io::Result<Self> {
        if len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "kvm_run mmap length must be non-zero",
            ));
        }

        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        let ptr =
            NonNull::new(raw).ok_or_else(|| io::Error::other("mmap unexpectedly returned null"))?;
        Ok(Self { ptr, len })
    }
}

impl Drop for KvmRunMapping {
    fn drop(&mut self) {
        let result = unsafe { libc::munmap(self.ptr.as_ptr(), self.len) };
        debug_assert_eq!(result, 0, "munmap(kvm_run) failed during Drop");
    }
}
