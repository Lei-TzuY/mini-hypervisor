use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    HostEnvironment(HostEnvironmentError),
    KvmCapability(KvmCapabilityError),
    Configuration(ConfigurationError),
    GuestMemory(GuestMemoryError),
}

#[derive(Debug)]
pub enum HostEnvironmentError {
    KvmUnavailable {
        source: io::Error,
    },
    PermissionDenied {
        source: io::Error,
    },
    VmCreation {
        source: io::Error,
    },
    VcpuCreation {
        id: u16,
        source: io::Error,
    },
    VcpuRunMapping {
        id: u16,
        source: io::Error,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KvmCapabilityError {
    UnsupportedApiVersion { expected: i32, actual: i32 },
    MissingExtension { name: &'static str, id: i32 },
    InvalidVcpuMmapSize { size: i32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationError {
    UnsupportedVcpuCount { requested: u16, supported: u16 },
}

#[derive(Debug)]
pub enum GuestMemoryError {
    ZeroSizedRegion,
    MisalignedRegion {
        field: &'static str,
        value: u64,
        alignment: u64,
    },
    AddressSpaceOverflow {
        base: u64,
        size: u64,
    },
    HostSizeOverflow {
        size: u64,
    },
    AccessLengthTooLarge {
        length: usize,
    },
    AccessOverflow {
        address: u64,
        length: usize,
    },
    AccessOutOfBounds {
        address: u64,
        length: usize,
        region_base: u64,
        region_size: u64,
    },
    Mapping {
        source: io::Error,
    },
    Registration {
        source: io::Error,
    },
    AlreadyRegistered,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostEnvironment(error) => error.fmt(f),
            Self::KvmCapability(error) => error.fmt(f),
            Self::Configuration(error) => error.fmt(f),
            Self::GuestMemory(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::HostEnvironment(error) => error.source(),
            Self::GuestMemory(error) => error.source(),
            Self::KvmCapability(_) | Self::Configuration(_) => None,
        }
    }
}

impl fmt::Display for HostEnvironmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KvmUnavailable { .. } => write!(f, "/dev/kvm is unavailable"),
            Self::PermissionDenied { .. } => write!(f, "permission denied while opening /dev/kvm"),
            Self::VmCreation { .. } => write!(f, "KVM failed to create a VM"),
            Self::VcpuCreation { id, .. } => write!(f, "KVM failed to create vCPU {id}"),
            Self::VcpuRunMapping { id, .. } => {
                write!(f, "failed to map the kvm_run structure for vCPU {id}")
            }
            Self::Io { operation, .. } => write!(f, "host I/O failure during {operation}"),
        }
    }
}

impl std::error::Error for HostEnvironmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::KvmUnavailable { source }
            | Self::PermissionDenied { source }
            | Self::VmCreation { source }
            | Self::VcpuCreation { source, .. }
            | Self::VcpuRunMapping { source, .. }
            | Self::Io { source, .. } => Some(source),
        }
    }
}

impl fmt::Display for KvmCapabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedApiVersion { expected, actual } => write!(
                f,
                "unsupported KVM API version: expected {expected}, got {actual}"
            ),
            Self::MissingExtension { name, id } => {
                write!(f, "required KVM extension {name} (id {id}) is unavailable")
            }
            Self::InvalidVcpuMmapSize { size } => {
                write!(f, "KVM reported invalid vCPU mmap size {size}")
            }
        }
    }
}

impl fmt::Display for ConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVcpuCount {
                requested,
                supported,
            } => write!(
                f,
                "requested {requested} vCPUs, but this milestone supports exactly {supported}"
            ),
        }
    }
}

impl fmt::Display for GuestMemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroSizedRegion => write!(f, "guest RAM region must be non-zero"),
            Self::MisalignedRegion {
                field,
                value,
                alignment,
            } => write!(
                f,
                "guest RAM {field} {value:#x} is not aligned to {alignment:#x} bytes"
            ),
            Self::AddressSpaceOverflow { base, size } => write!(
                f,
                "guest RAM range overflows the physical address space: base={base:#x}, size={size:#x}"
            ),
            Self::HostSizeOverflow { size } => {
                write!(f, "guest RAM size {size:#x} does not fit the host address space")
            }
            Self::AccessLengthTooLarge { length } => {
                write!(f, "guest-memory access length {length} does not fit in a guest address")
            }
            Self::AccessOverflow { address, length } => write!(
                f,
                "guest-memory access overflows: address={address:#x}, length={length}"
            ),
            Self::AccessOutOfBounds {
                address,
                length,
                region_base,
                region_size,
            } => write!(
                f,
                "guest-memory access is outside RAM: address={address:#x}, length={length}, region={region_base:#x}+{region_size:#x}"
            ),
            Self::Mapping { .. } => write!(f, "failed to map guest RAM on the host"),
            Self::Registration { .. } => write!(f, "KVM failed to register guest RAM"),
            Self::AlreadyRegistered => write!(f, "this VM already owns its single guest RAM region"),
        }
    }
}

impl std::error::Error for GuestMemoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mapping { source } | Self::Registration { source } => Some(source),
            Self::ZeroSizedRegion
            | Self::MisalignedRegion { .. }
            | Self::AddressSpaceOverflow { .. }
            | Self::HostSizeOverflow { .. }
            | Self::AccessLengthTooLarge { .. }
            | Self::AccessOverflow { .. }
            | Self::AccessOutOfBounds { .. }
            | Self::AlreadyRegistered => None,
        }
    }
}
