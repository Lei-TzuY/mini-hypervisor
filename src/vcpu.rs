use crate::error::{ConfigurationError, Error, HostEnvironmentError};
use crate::kvm::sys;
use crate::memory::GuestPhysAddr;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr::NonNull;

const REAL_MODE_MAX_RIP: u64 = u16::MAX as u64;
const CR0_PROTECTED_MODE_ENABLE: u64 = 1;
const CR0_PAGING_ENABLE: u64 = 1 << 31;
const RFLAGS_RESERVED_BIT: u64 = 1 << 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcpuExit {
    Hlt,
    Unhandled { reason: u32 },
}

impl VcpuExit {
    #[must_use]
    pub const fn from_raw(reason: u32) -> Self {
        match reason {
            sys::KVM_EXIT_HLT => Self::Hlt,
            _ => Self::Unhandled { reason },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuRegisters {
    pub rip: u64,
    pub rflags: u64,
}

#[derive(Debug)]
pub struct Vcpu {
    id: VcpuId,
    fd: OwnedFd,
    run: KvmRunMapping,
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
        Ok(Self { id, fd, run })
    }

    #[must_use]
    pub const fn id(&self) -> VcpuId {
        self.id
    }

    pub fn initialize_real_mode(&self, entry: GuestPhysAddr) -> Result<(), Error> {
        let rip = validate_real_mode_entry(entry)?;
        let mut sregs = sys::get_sregs(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_SREGS", source))?;

        initialize_real_mode_segment(&mut sregs.cs);
        initialize_real_mode_segment(&mut sregs.ds);
        initialize_real_mode_segment(&mut sregs.es);
        initialize_real_mode_segment(&mut sregs.fs);
        initialize_real_mode_segment(&mut sregs.gs);
        initialize_real_mode_segment(&mut sregs.ss);
        sregs.cr0 &= !(CR0_PROTECTED_MODE_ENABLE | CR0_PAGING_ENABLE);

        sys::set_sregs(self.fd.as_raw_fd(), &sregs)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_SREGS", source))?;

        let regs = sys::KvmRegs {
            rip,
            rflags: RFLAGS_RESERVED_BIT,
            ..sys::KvmRegs::default()
        };
        sys::set_regs(self.fd.as_raw_fd(), &regs)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_REGS", source))?;

        Ok(())
    }

    pub fn registers(&self) -> Result<VcpuRegisters, Error> {
        let regs = sys::get_regs(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_REGS", source))?;
        Ok(VcpuRegisters {
            rip: regs.rip,
            rflags: regs.rflags,
        })
    }

    pub fn run_once(&mut self) -> Result<VcpuExit, Error> {
        loop {
            match sys::run_vcpu(self.fd.as_raw_fd()) {
                Ok(()) => break,
                Err(source) if source.kind() == io::ErrorKind::Interrupted => continue,
                Err(source) => return Err(vcpu_operation(self.id, "KVM_RUN", source)),
            }
        }

        Ok(VcpuExit::from_raw(self.run.exit_reason()))
    }
}

fn validate_real_mode_entry(entry: GuestPhysAddr) -> Result<u64, Error> {
    if entry.get() > REAL_MODE_MAX_RIP {
        return Err(Error::Configuration(
            ConfigurationError::RealModeEntryOutOfRange {
                entry: entry.get(),
                maximum: REAL_MODE_MAX_RIP,
            },
        ));
    }
    Ok(entry.get())
}

fn initialize_real_mode_segment(segment: &mut sys::KvmSegment) {
    segment.base = 0;
    segment.selector = 0;
}

fn vcpu_operation(id: VcpuId, operation: &'static str, source: io::Error) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: id.get(),
        operation,
        source,
    })
}

#[derive(Debug)]
struct KvmRunMapping {
    ptr: NonNull<libc::c_void>,
    len: usize,
}

impl KvmRunMapping {
    fn map(fd: &OwnedFd, len: usize) -> io::Result<Self> {
        if len < std::mem::size_of::<sys::KvmRunHeader>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "kvm_run mmap length is smaller than the required header",
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

    fn exit_reason(&self) -> u32 {
        // SAFETY: construction requires a mapping at least as large as `KvmRunHeader`, and KVM
        // defines that structure as the prefix at offset zero of the shared `kvm_run` mapping.
        let header = unsafe { &*self.ptr.as_ptr().cast::<sys::KvmRunHeader>() };
        header.exit_reason
    }
}

impl Drop for KvmRunMapping {
    fn drop(&mut self) {
        let result = unsafe { libc::munmap(self.ptr.as_ptr(), self.len) };
        debug_assert_eq!(result, 0, "munmap(kvm_run) failed during Drop");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_hlt_and_preserves_unknown_reason() {
        assert_eq!(VcpuExit::from_raw(sys::KVM_EXIT_HLT), VcpuExit::Hlt);
        assert_eq!(
            VcpuExit::from_raw(0xfeed_beef),
            VcpuExit::Unhandled {
                reason: 0xfeed_beef
            }
        );
    }

    #[test]
    fn accepts_current_real_mode_entry_range() {
        assert_eq!(
            validate_real_mode_entry(GuestPhysAddr::new(REAL_MODE_MAX_RIP)).unwrap(),
            REAL_MODE_MAX_RIP
        );
    }

    #[test]
    fn rejects_real_mode_entry_above_current_cs_zero_limit() {
        assert!(matches!(
            validate_real_mode_entry(GuestPhysAddr::new(REAL_MODE_MAX_RIP + 1)),
            Err(Error::Configuration(
                ConfigurationError::RealModeEntryOutOfRange { .. }
            ))
        ));
    }
}
