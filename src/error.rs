use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    HostEnvironment(HostEnvironmentError),
    KvmCapability(KvmCapabilityError),
    Configuration(ConfigurationError),
}

#[derive(Debug)]
pub enum HostEnvironmentError {
    KvmUnavailable { source: io::Error },
    PermissionDenied { source: io::Error },
    Io { operation: &'static str, source: io::Error },
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

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostEnvironment(error) => error.fmt(f),
            Self::KvmCapability(error) => error.fmt(f),
            Self::Configuration(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::HostEnvironment(error) => error.source(),
            Self::KvmCapability(_) | Self::Configuration(_) => None,
        }
    }
}

impl fmt::Display for HostEnvironmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KvmUnavailable { .. } => write!(f, "/dev/kvm is unavailable"),
            Self::PermissionDenied { .. } => write!(f, "permission denied while opening /dev/kvm"),
            Self::Io { operation, .. } => write!(f, "host I/O failure during {operation}"),
        }
    }
}

impl std::error::Error for HostEnvironmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::KvmUnavailable { source }
            | Self::PermissionDenied { source }
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
