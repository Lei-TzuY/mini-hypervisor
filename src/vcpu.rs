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
mod interrupt;
mod special_register_restore;

mod msr_readback;
pub use msr_readback::{VcpuMsrValue, VcpuMsrValues};

mod kvm_unknown;
pub use kvm_unknown::VcpuKvmUnknownExit;

mod exception;
pub use exception::VcpuException;

mod fail_entry;
pub use fail_entry::VcpuFailEntry;

mod internal_error;
pub use internal_error::{VcpuInternalError, VcpuInternalErrorSuberror};

mod system_event;
pub use system_event::{VcpuSystemEvent, VcpuSystemEventType};

mod mmio;
pub use mmio::{MmioDirection, MmioExit, KVM_MMIO_DATA_CAPACITY};

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
    KvmUnknown,
    Exception,
    Hlt,
    Io,
    Mmio,
    Shutdown,
    FailEntry,
    InternalError,
    SystemEvent,
    Unhandled { reason: u32 },
}

impl VcpuExit {
    #[must_use]
    pub const fn from_raw(reason: u32) -> Self {
        match reason {
            kvm_unknown::KVM_EXIT_UNKNOWN => Self::KvmUnknown,
            exception::KVM_EXIT_EXCEPTION => Self::Exception,
            sys::KVM_EXIT_IO => Self::Io,
            sys::KVM_EXIT_HLT => Self::Hlt,
            mmio::KVM_EXIT_MMIO => Self::Mmio,
            sys::KVM_EXIT_SHUTDOWN => Self::Shutdown,
            fail_entry::KVM_EXIT_FAIL_ENTRY => Self::FailEntry,
            internal_error::KVM_EXIT_INTERNAL_ERROR => Self::InternalError,
            system_event::KVM_EXIT_SYSTEM_EVENT => Self::SystemEvent,
            _ => Self::Unhandled { reason },
        }
    }

    #[must_use]
    pub const fn reason(self) -> u32 {
        match self {
            Self::KvmUnknown => kvm_unknown::KVM_EXIT_UNKNOWN,
            Self::Exception => exception::KVM_EXIT_EXCEPTION,
            Self::Io => sys::KVM_EXIT_IO,
            Self::Hlt => sys::KVM_EXIT_HLT,
            Self::Mmio => mmio::KVM_EXIT_MMIO,
            Self::Shutdown => sys::KVM_EXIT_SHUTDOWN,
            Self::FailEntry => fail_entry::KVM_EXIT_FAIL_ENTRY,
            Self::InternalError => internal_error::KVM_EXIT_INTERNAL_ERROR,
            Self::SystemEvent => system_event::KVM_EXIT_SYSTEM_EVENT,
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
    direction: PortIoDirection,
    size: u8,
    port: u16,
    count: u32,
    output_data: Vec<u8>,
}

impl PortIoExit {
    pub(crate) fn new(
        direction: PortIoDirection,
        size: u8,
        port: u16,
        count: u32,
        output_data: Vec<u8>,
    ) -> Self {
        Self {
            direction,
            size,
            port,
            count,
            output_data,
        }
    }

    #[must_use]
    pub const fn direction(&self) -> PortIoDirection {
        self.direction
    }

    #[must_use]
    pub const fn size(&self) -> u8 {
        self.size
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    #[must_use]
    pub fn output_data(&self) -> &[u8] {
        &self.output_data
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
    supports_internal_error_data: bool,
}

impl Vcpu {
    pub(crate) fn from_kvm_fd(
        id: VcpuId,
        fd: OwnedFd,
        run_mmap_size: usize,
        supports_internal_error_data: bool,
    ) -> Result<Self, Error> {
        let run = KvmRunMapping::map(&fd, run_mmap_size).map_err(|source| {
            Error::HostEnvironment(HostEnvironmentError::VcpuRunMapping {
                id: id.get(),
                source,
            })
        })?;
        Ok(Self {
            id,
            fd,
            run,
            supports_internal_error_data,
        })
    }

    #[must_use]
    pub const fn id(&self) -> VcpuId {
        self.id
    }

    #[must_use]
    pub const fn supports_internal_error_data(&self) -> bool {
        self.supports_internal_error_data
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

    pub(crate) fn port_io_exit(&self) -> Result<PortIoExit, Error> {
        self.run.port_io_exit(self.id)
    }

    pub(crate) fn write_port_io_input(&mut self, response: &[u8]) -> Result<(), Error> {
        self.run.write_port_io_input(self.id, response)
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

fn port_io_direction(id: VcpuId, raw: u8) -> Result<PortIoDirection, Error> {
    match raw {
        sys::KVM_EXIT_IO_IN => Ok(PortIoDirection::In),
        sys::KVM_EXIT_IO_OUT => Ok(PortIoDirection::Out),
        direction => Err(Error::VmExit(VmExitError::InvalidIoDirection {
            vcpu_id: id.get(),
            direction,
        })),
    }
}

fn checked_io_data_range(
    id: VcpuId,
    io: sys::KvmRunIo,
    mapping_size: usize,
) -> Result<Range<usize>, Error> {
    let invalid_range = || {
        Error::VmExit(VmExitError::InvalidIoDataRange {
            vcpu_id: id.get(),
            data_offset: io.data_offset,
            size: io.size,
            count: io.count,
            mapping_size,
        })
    };

    let offset = usize::try_from(io.data_offset).map_err(|_| invalid_range())?;
    let count = usize::try_from(io.count).map_err(|_| invalid_range())?;
    let length = usize::from(io.size)
        .checked_mul(count)
        .ok_or_else(&invalid_range)?;
    let end = offset.checked_add(length).ok_or_else(&invalid_range)?;
    if end > mapping_size {
        return Err(invalid_range());
    }

    Ok(offset..end)
}

fn checked_io_response_range(
    id: VcpuId,
    io: sys::KvmRunIo,
    mapping_size: usize,
    response_len: usize,
) -> Result<Range<usize>, Error> {
    let direction = port_io_direction(id, io.direction)?;
    if direction != PortIoDirection::In {
        return Err(Error::VmExit(VmExitError::IoResponseForNonInput {
            vcpu_id: id.get(),
            direction: io.direction,
        }));
    }

    let range = checked_io_data_range(id, io, mapping_size)?;
    if response_len != range.len() {
        return Err(Error::VmExit(VmExitError::InvalidIoResponseLength {
            vcpu_id: id.get(),
            port: io.port,
            expected: range.len(),
            actual: response_len,
        }));
    }

    Ok(range)
}

fn required_kvm_run_prefix_size() -> usize {
    std::mem::size_of::<sys::KvmRunIoPrefix>()
        .max(kvm_unknown::required_kvm_run_prefix_size())
        .max(exception::required_kvm_run_prefix_size())
        .max(fail_entry::required_kvm_run_prefix_size())
        .max(internal_error::required_kvm_run_prefix_size())
        .max(system_event::required_kvm_run_prefix_size())
        .max(mmio::required_kvm_run_prefix_size())
}

#[derive(Debug)]
struct KvmRunMapping {
    ptr: NonNull<libc::c_void>,
    len: usize,
}

impl KvmRunMapping {
    fn map(fd: &OwnedFd, len: usize) -> io::Result<Self> {
        if len < required_kvm_run_prefix_size() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "kvm_run mmap length is smaller than the required x86 exit payload prefix",
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

    fn prefix(&self) -> &sys::KvmRunIoPrefix {
        // SAFETY: construction requires a mapping at least as large as `KvmRunIoPrefix`, KVM
        // places `struct kvm_run` at offset zero, and mmap returns suitably aligned memory.
        unsafe { &*self.ptr.as_ptr().cast::<sys::KvmRunIoPrefix>() }
    }

    fn exit_reason(&self) -> u32 {
        self.prefix().header.exit_reason
    }

    fn port_io_exit(&self, id: VcpuId) -> Result<PortIoExit, Error> {
        let prefix = self.prefix();
        if prefix.header.exit_reason != sys::KVM_EXIT_IO {
            return Err(Error::VmExit(VmExitError::IoPayloadUnavailable {
                vcpu_id: id.get(),
                exit_reason: prefix.header.exit_reason,
            }));
        }

        let io = prefix.io;
        let direction = port_io_direction(id, io.direction)?;
        let range = checked_io_data_range(id, io, self.len)?;
        let output_data = if direction == PortIoDirection::Out {
            let mut bytes = vec![0; range.len()];
            if !bytes.is_empty() {
                // SAFETY: `range` was checked against the mapping length. The destination owns
                // `range.len()` initialized bytes and cannot overlap the mmap source.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        self.ptr.as_ptr().cast::<u8>().add(range.start),
                        bytes.as_mut_ptr(),
                        range.len(),
                    );
                }
            }
            bytes
        } else {
            Vec::new()
        };

        Ok(PortIoExit::new(
            direction,
            io.size,
            io.port,
            io.count,
            output_data,
        ))
    }

    fn write_port_io_input(&mut self, id: VcpuId, response: &[u8]) -> Result<(), Error> {
        let prefix = self.prefix();
        if prefix.header.exit_reason != sys::KVM_EXIT_IO {
            return Err(Error::VmExit(VmExitError::IoPayloadUnavailable {
                vcpu_id: id.get(),
                exit_reason: prefix.header.exit_reason,
            }));
        }

        let io = prefix.io;
        let range = checked_io_response_range(id, io, self.len, response.len())?;
        if response.is_empty() {
            return Ok(());
        }

        // SAFETY: `checked_io_response_range` proves the complete destination range lies inside
        // this writable shared `kvm_run` mapping. The response slice is valid and non-overlapping.
        unsafe {
            std::ptr::copy_nonoverlapping(
                response.as_ptr(),
                self.ptr.as_ptr().cast::<u8>().add(range.start),
                response.len(),
            );
        }
        Ok(())
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
    fn classifies_typed_exits_and_preserves_other_unhandled_reason() {
        assert_eq!(
            VcpuExit::from_raw(kvm_unknown::KVM_EXIT_UNKNOWN),
            VcpuExit::KvmUnknown
        );
        assert_eq!(
            VcpuExit::from_raw(exception::KVM_EXIT_EXCEPTION),
            VcpuExit::Exception
        );
        assert_eq!(VcpuExit::from_raw(sys::KVM_EXIT_HLT), VcpuExit::Hlt);
        assert_eq!(VcpuExit::from_raw(sys::KVM_EXIT_IO), VcpuExit::Io);
        assert_eq!(VcpuExit::from_raw(mmio::KVM_EXIT_MMIO), VcpuExit::Mmio);
        assert_eq!(
            VcpuExit::from_raw(sys::KVM_EXIT_SHUTDOWN),
            VcpuExit::Shutdown
        );
        assert_eq!(
            VcpuExit::from_raw(fail_entry::KVM_EXIT_FAIL_ENTRY),
            VcpuExit::FailEntry
        );
        assert_eq!(
            VcpuExit::from_raw(internal_error::KVM_EXIT_INTERNAL_ERROR),
            VcpuExit::InternalError
        );
        assert_eq!(
            VcpuExit::from_raw(system_event::KVM_EXIT_SYSTEM_EVENT),
            VcpuExit::SystemEvent
        );
        assert_eq!(
            VcpuExit::from_raw(0xfeed_beef),
            VcpuExit::Unhandled {
                reason: 0xfeed_beef
            }
        );
    }

    #[test]
    fn exit_reason_round_trips_typed_classification() {
        assert_eq!(VcpuExit::KvmUnknown.reason(), kvm_unknown::KVM_EXIT_UNKNOWN);
        assert_eq!(VcpuExit::Exception.reason(), exception::KVM_EXIT_EXCEPTION);
        assert_eq!(VcpuExit::Hlt.reason(), sys::KVM_EXIT_HLT);
        assert_eq!(VcpuExit::Io.reason(), sys::KVM_EXIT_IO);
        assert_eq!(VcpuExit::Mmio.reason(), mmio::KVM_EXIT_MMIO);
        assert_eq!(VcpuExit::Shutdown.reason(), sys::KVM_EXIT_SHUTDOWN);
        assert_eq!(
            VcpuExit::FailEntry.reason(),
            fail_entry::KVM_EXIT_FAIL_ENTRY
        );
        assert_eq!(
            VcpuExit::InternalError.reason(),
            internal_error::KVM_EXIT_INTERNAL_ERROR
        );
        assert_eq!(
            VcpuExit::SystemEvent.reason(),
            system_event::KVM_EXIT_SYSTEM_EVENT
        );
        assert_eq!(VcpuExit::Unhandled { reason: 0x1234 }.reason(), 0x1234);
    }

    #[test]
    fn required_run_prefix_covers_all_typed_payloads() {
        assert!(required_kvm_run_prefix_size() >= std::mem::size_of::<sys::KvmRunIoPrefix>());
        assert!(required_kvm_run_prefix_size() >= kvm_unknown::required_kvm_run_prefix_size());
        assert!(required_kvm_run_prefix_size() >= exception::required_kvm_run_prefix_size());
        assert!(required_kvm_run_prefix_size() >= fail_entry::required_kvm_run_prefix_size());
        assert!(required_kvm_run_prefix_size() >= internal_error::required_kvm_run_prefix_size());
        assert!(required_kvm_run_prefix_size() >= system_event::required_kvm_run_prefix_size());
        assert!(required_kvm_run_prefix_size() >= mmio::required_kvm_run_prefix_size());
    }

    #[test]
    fn validates_port_io_directions() {
        assert_eq!(
            port_io_direction(VcpuId::BOOT, sys::KVM_EXIT_IO_IN).unwrap(),
            PortIoDirection::In
        );
        assert_eq!(
            port_io_direction(VcpuId::BOOT, sys::KVM_EXIT_IO_OUT).unwrap(),
            PortIoDirection::Out
        );
        assert!(matches!(
            port_io_direction(VcpuId::new(3), 9),
            Err(Error::VmExit(VmExitError::InvalidIoDirection {
                vcpu_id: 3,
                direction: 9,
            }))
        ));
    }

    #[test]
    fn validates_port_io_data_range() {
        let io = sys::KvmRunIo {
            direction: sys::KVM_EXIT_IO_OUT,
            size: 2,
            port: 0xe9,
            count: 3,
            data_offset: 48,
        };
        assert_eq!(checked_io_data_range(VcpuId::BOOT, io, 64).unwrap(), 48..54);
    }

    #[test]
    fn validates_exact_port_io_input_response_range() {
        let io = sys::KvmRunIo {
            direction: sys::KVM_EXIT_IO_IN,
            size: 1,
            port: 0xe9,
            count: 1,
            data_offset: 48,
        };

        assert_eq!(
            checked_io_response_range(VcpuId::BOOT, io, 64, 1).unwrap(),
            48..49
        );
    }

    #[test]
    fn rejects_port_io_response_for_output_exit() {
        let io = sys::KvmRunIo {
            direction: sys::KVM_EXIT_IO_OUT,
            size: 1,
            port: 0xe9,
            count: 1,
            data_offset: 48,
        };

        assert!(matches!(
            checked_io_response_range(VcpuId::new(4), io, 64, 1),
            Err(Error::VmExit(VmExitError::IoResponseForNonInput {
                vcpu_id: 4,
                direction: sys::KVM_EXIT_IO_OUT,
            }))
        ));
    }

    #[test]
    fn rejects_port_io_input_response_length_mismatch() {
        let io = sys::KvmRunIo {
            direction: sys::KVM_EXIT_IO_IN,
            size: 2,
            port: 0xe9,
            count: 2,
            data_offset: 48,
        };

        assert!(matches!(
            checked_io_response_range(VcpuId::new(5), io, 64, 3),
            Err(Error::VmExit(VmExitError::InvalidIoResponseLength {
                vcpu_id: 5,
                port: 0xe9,
                expected: 4,
                actual: 3,
            }))
        ));
    }

    #[test]
    fn rejects_port_io_data_range_outside_mapping() {
        let io = sys::KvmRunIo {
            direction: sys::KVM_EXIT_IO_OUT,
            size: 4,
            port: 0xe9,
            count: 2,
            data_offset: 60,
        };
        assert!(matches!(
            checked_io_data_range(VcpuId::new(2), io, 64),
            Err(Error::VmExit(VmExitError::InvalidIoDataRange {
                vcpu_id: 2,
                data_offset: 60,
                size: 4,
                count: 2,
                mapping_size: 64,
            }))
        ));
    }

    #[test]
    fn rejects_port_io_data_range_overflow() {
        let io = sys::KvmRunIo {
            direction: sys::KVM_EXIT_IO_OUT,
            size: 2,
            port: 0xe9,
            count: 1,
            data_offset: u64::MAX,
        };
        assert!(matches!(
            checked_io_data_range(VcpuId::BOOT, io, usize::MAX),
            Err(Error::VmExit(VmExitError::InvalidIoDataRange { .. }))
        ));
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
