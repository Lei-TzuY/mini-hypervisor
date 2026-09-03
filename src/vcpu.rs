use crate::error::{ConfigurationError, Error, HostEnvironmentError, VmExitError};
use crate::kvm::sys;
use crate::memory::GuestPhysAddr;
use std::io;
use std::ops::Range;
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr::NonNull;

mod register_snapshot;
pub use register_snapshot::{
    VcpuRegisterField, VcpuRegisterMismatch, VcpuRegisterSnapshot, VcpuRegisterSnapshotComparison,
};

mod special_register_snapshot;
pub use special_register_snapshot::{
    VcpuDescriptorTableField, VcpuDescriptorTableRegister, VcpuDescriptorTableState,
    VcpuInterruptBitmapWord, VcpuSegmentField, VcpuSegmentRegister, VcpuSegmentState,
    VcpuSpecialRegisterField, VcpuSpecialRegisterMismatch, VcpuSpecialRegisterSnapshot,
    VcpuSpecialRegisterSnapshotComparison,
};

mod msr_readback;
pub use msr_readback::{VcpuMsrValue, VcpuMsrValues};

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
    Io,
    Unhandled { reason: u32 },
}

impl VcpuExit {
    #[must_use]
    pub const fn from_raw(reason: u32) -> Self {
        match reason {
            sys::KVM_EXIT_IO => Self::Io,
            sys::KVM_EXIT_HLT => Self::Hlt,
            _ => Self::Unhandled { reason },
        }
    }

    #[must_use]
    pub const fn reason(self) -> u32 {
        match self {
            Self::Io => sys::KVM_EXIT_IO,
            Self::Hlt => sys::KVM_EXIT_HLT,
            Self::Unhandled { reason } => reason,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortIoDirection {
    In,
    Out,
}

impl PortIoDirection {
    #[must_use]
    pub const fn raw(self) -> u8 {
        match self {
            Self::In => sys::KVM_EXIT_IO_IN,
            Self::Out => sys::KVM_EXIT_IO_OUT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortIoExit {
    pub direction: PortIoDirection,
    pub port: u16,
    pub size: u8,
    pub count: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpuRegisters {
    pub rip: u64,
    pub rflags: u64,
}

pub struct Vcpu {
    id: VcpuId,
    fd: OwnedFd,
    run: NonNull<u8>,
    run_size: usize,
}

impl Vcpu {
    pub(crate) fn new(id: VcpuId, fd: OwnedFd, run: NonNull<u8>, run_size: usize) -> Self {
        Self {
            id,
            fd,
            run,
            run_size,
        }
    }

    #[must_use]
    pub const fn id(&self) -> VcpuId {
        self.id
    }

    pub fn initialize_real_mode(&self, entry: GuestPhysAddr) -> Result<(), Error> {
        let rip = entry.raw();
        if rip > REAL_MODE_MAX_RIP {
            return Err(ConfigurationError::RealModeEntryOutOfRange { entry: rip }.into());
        }

        let mut sregs = sys::get_sregs(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_SREGS", source))?;
        for segment in [
            &mut sregs.cs,
            &mut sregs.ds,
            &mut sregs.es,
            &mut sregs.fs,
            &mut sregs.gs,
            &mut sregs.ss,
        ] {
            segment.base = 0;
            segment.selector = 0;
        }
        sregs.cr0 &= !(CR0_PROTECTED_MODE_ENABLE | CR0_PAGING_ENABLE);
        sys::set_sregs(self.fd.as_raw_fd(), &sregs)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_SREGS", source))?;

        let regs = sys::KvmRegs {
            rip,
            rflags: RFLAGS_RESERVED_BIT,
            ..sys::KvmRegs::default()
        };
        sys::set_regs(self.fd.as_raw_fd(), &regs)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_REGS", source))
    }

    pub fn registers(&self) -> Result<VcpuRegisters, Error> {
        let regs = sys::get_regs(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_REGS", source))?;
        Ok(VcpuRegisters {
            rip: regs.rip,
            rflags: regs.rflags,
        })
    }

    pub fn run(&mut self) -> Result<VcpuExit, Error> {
        sys::run_vcpu(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_RUN", source))?;
        let reason = self.run_header().exit_reason;
        Ok(VcpuExit::from_raw(reason))
    }

    pub fn port_io_exit(&self) -> Result<PortIoExit, Error> {
        let reason = self.run_header().exit_reason;
        if reason != sys::KVM_EXIT_IO {
            return Err(VmExitError::ExpectedPortIo { reason }.into());
        }

        let raw = unsafe { self.run_header().exit.io };
        let direction = match raw.direction {
            sys::KVM_EXIT_IO_IN => PortIoDirection::In,
            sys::KVM_EXIT_IO_OUT => PortIoDirection::Out,
            value => return Err(VmExitError::InvalidPortIoDirection { value }.into()),
        };
        let range = checked_port_io_range(self.run_size, raw.data_offset, raw.size, raw.count)?;
        let data = unsafe { self.run_bytes(range.clone()) }.to_vec();

        Ok(PortIoExit {
            direction,
            port: raw.port,
            size: raw.size,
            count: raw.count,
            data,
        })
    }

    pub fn complete_port_io_input(&mut self, response: &[u8]) -> Result<(), Error> {
        let reason = self.run_header().exit_reason;
        if reason != sys::KVM_EXIT_IO {
            return Err(VmExitError::ExpectedPortIo { reason }.into());
        }

        let raw = unsafe { self.run_header().exit.io };
        if raw.direction != sys::KVM_EXIT_IO_IN {
            return Err(VmExitError::PortIoInputResponseForNonInput {
                direction: raw.direction,
            }
            .into());
        }
        let range = checked_port_io_range(self.run_size, raw.data_offset, raw.size, raw.count)?;
        if response.len() != range.len() {
            return Err(VmExitError::InvalidPortIoInputResponseLength {
                expected: range.len(),
                actual: response.len(),
            }
            .into());
        }

        unsafe { self.run_bytes_mut(range) }.copy_from_slice(response);
        Ok(())
    }

    fn run_header(&self) -> &sys::KvmRun {
        unsafe { self.run.as_ref().cast::<sys::KvmRun>() }
    }

    unsafe fn run_bytes(&self, range: Range<usize>) -> &[u8] {
        std::slice::from_raw_parts(self.run.as_ptr().add(range.start), range.len())
    }

    unsafe fn run_bytes_mut(&mut self, range: Range<usize>) -> &mut [u8] {
        std::slice::from_raw_parts_mut(self.run.as_ptr().add(range.start), range.len())
    }
}

impl Drop for Vcpu {
    fn drop(&mut self) {
        let result = unsafe { libc::munmap(self.run.as_ptr().cast(), self.run_size) };
        debug_assert_eq!(result, 0, "munmap(kvm_run) failed during vCPU drop");
    }
}

fn checked_port_io_range(
    run_size: usize,
    data_offset: u64,
    size: u8,
    count: u32,
) -> Result<Range<usize>, Error> {
    let start = usize::try_from(data_offset)
        .map_err(|_| VmExitError::PortIoDataOffsetOutOfRange { data_offset })?;
    let access_size = usize::from(size);
    let count = usize::try_from(count)
        .map_err(|_| VmExitError::PortIoLengthOverflow { size, count })?;
    let length = access_size
        .checked_mul(count)
        .ok_or(VmExitError::PortIoLengthOverflow {
            size,
            count: u32::try_from(count).unwrap_or(u32::MAX),
        })?;
    let end = start
        .checked_add(length)
        .ok_or(VmExitError::PortIoRangeOverflow {
            data_offset,
            length,
        })?;
    if end > run_size {
        return Err(VmExitError::PortIoRangeOutOfBounds {
            data_offset,
            length,
            run_size,
        }
        .into());
    }
    Ok(start..end)
}

pub(crate) fn vcpu_operation(id: VcpuId, operation: &'static str, source: io::Error) -> Error {
    HostEnvironmentError::VcpuOperation {
        vcpu: id.get(),
        operation,
        source,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_exit_reasons_are_typed() {
        assert_eq!(VcpuExit::from_raw(sys::KVM_EXIT_HLT), VcpuExit::Hlt);
        assert_eq!(VcpuExit::from_raw(sys::KVM_EXIT_IO), VcpuExit::Io);
    }

    #[test]
    fn unknown_exit_reason_is_preserved() {
        assert_eq!(
            VcpuExit::from_raw(0xfeed),
            VcpuExit::Unhandled { reason: 0xfeed }
        );
    }

    #[test]
    fn port_io_range_is_checked() {
        assert_eq!(checked_port_io_range(4096, 128, 4, 3).unwrap(), 128..140);
    }

    #[test]
    fn port_io_range_rejects_out_of_bounds() {
        let error = checked_port_io_range(4096, 4090, 4, 2).unwrap_err();
        assert!(matches!(
            error,
            Error::VmExit(VmExitError::PortIoRangeOutOfBounds { .. })
        ));
    }
}
