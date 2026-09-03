pub mod cpu;
pub mod msr;
pub(crate) mod sys;

use crate::error::{Error, GuestMemoryError, HostEnvironmentError, KvmCapabilityError};
use crate::memory::{GuestMemory, GuestMemoryRegion, GuestPhysAddr, KVM_MEMORY_ALIGNMENT};
use crate::vcpu::{Vcpu, VcpuId};
use cpu::{CpuidEntry, GuestCpuPolicy, HostCpuid};
use msr::{
    HostMsrFeatureIndexList, HostMsrFeatureValues, HostMsrIndexList, MsrFeatureValue, MsrIndex,
};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

const EXPECTED_KVM_API_VERSION: i32 = 12;
const KVM_CAP_USER_MEMORY: i32 = 3;
const KVM_CAP_SET_TSS_ADDR: i32 = 4;
const KVM_CAP_EXT_CPUID: i32 = 7;
const KVM_CAP_SET_IDENTITY_MAP_ADDR: i32 = 37;
const KVM_CAP_INTERNAL_ERROR_DATA: i32 = 40;
const KVM_CAP_GET_MSR_FEATURES: i32 = 153;
const KVM_IDENTITY_MAP_ADDR: u64 = 0xfeff_c000;
const KVM_TSS_ADDR: u64 = KVM_IDENTITY_MAP_ADDR + KVM_MEMORY_ALIGNMENT;
const KVM_RESERVED_X86_SIZE: u64 = 4 * KVM_MEMORY_ALIGNMENT;
const KVM_MAX_SUPPORTED_CPUID_ENTRIES: usize = 256;
const KVM_MAX_MSR_INDEX_LIST_ENTRIES: usize = 1024;

const REQUIRED_EXTENSIONS: [(&str, i32); 5] = [
    ("KVM_CAP_USER_MEMORY", KVM_CAP_USER_MEMORY),
    ("KVM_CAP_SET_TSS_ADDR", KVM_CAP_SET_TSS_ADDR),
    ("KVM_CAP_EXT_CPUID", KVM_CAP_EXT_CPUID),
    (
        "KVM_CAP_SET_IDENTITY_MAP_ADDR",
        KVM_CAP_SET_IDENTITY_MAP_ADDR,
    ),
    ("KVM_CAP_GET_MSR_FEATURES", KVM_CAP_GET_MSR_FEATURES),
];

const OPTIONAL_EXTENSIONS: [(&str, i32); 1] =
    [("KVM_CAP_INTERNAL_ERROR_DATA", KVM_CAP_INTERNAL_ERROR_DATA)];

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
    #[must_use]
    pub fn internal_error_data_capability(&self) -> Option<Capability> {
        self.extensions
            .iter()
            .copied()
            .find(|capability| capability.id == KVM_CAP_INTERNAL_ERROR_DATA)
    }

    #[must_use]
    pub fn supports_internal_error_data(&self) -> bool {
        self.internal_error_data_capability()
            .is_some_and(|capability| capability.value > 0)
    }

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

        for (name, id) in REQUIRED_EXTENSIONS {
            let available = self
                .extensions
                .iter()
                .any(|capability| capability.id == id && capability.value > 0);
            if !available {
                return Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
                    name,
                    id,
                }));
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct KvmBackend {
    fd: File,
    capabilities: HostCapabilities,
    host_cpuid: HostCpuid,
    host_msr_indices: HostMsrIndexList,
    host_msr_feature_indices: HostMsrFeatureIndexList,
    host_msr_feature_values: HostMsrFeatureValues,
    cpu_policy: GuestCpuPolicy,
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

        let mut extensions =
            Vec::with_capacity(REQUIRED_EXTENSIONS.len() + OPTIONAL_EXTENSIONS.len());
        for (name, id) in REQUIRED_EXTENSIONS {
            extensions.push(check_extension(&fd, name, id)?);
        }
        for (name, id) in OPTIONAL_EXTENSIONS {
            extensions.push(check_extension(&fd, name, id)?);
        }

        let capabilities = HostCapabilities {
            api_version,
            vcpu_mmap_size,
            extensions,
        };
        capabilities.validate()?;
        let host_cpuid = query_host_cpuid(&fd)?;
        let host_msr_indices = query_host_msr_indices(&fd)?;
        let host_msr_feature_indices = query_host_msr_feature_indices(&fd)?;
        let host_msr_feature_values =
            query_host_msr_feature_values(&fd, &host_msr_feature_indices)?;
        let cpu_policy = GuestCpuPolicy::from_host(&host_cpuid);

        Ok(Self {
            fd,
            capabilities,
            host_cpuid,
            host_msr_indices,
            host_msr_feature_indices,
            host_msr_feature_values,
            cpu_policy,
        })
    }

    #[must_use]
    pub fn capabilities(&self) -> &HostCapabilities {
        &self.capabilities
    }

    #[must_use]
    pub fn host_cpuid(&self) -> &HostCpuid {
        &self.host_cpuid
    }

    #[must_use]
    pub fn host_msr_indices(&self) -> &HostMsrIndexList {
        &self.host_msr_indices
    }

    #[must_use]
    pub fn host_msr_feature_indices(&self) -> &HostMsrFeatureIndexList {
        &self.host_msr_feature_indices
    }

    #[must_use]
    pub fn host_msr_feature_values(&self) -> &HostMsrFeatureValues {
        &self.host_msr_feature_values
    }

    #[must_use]
    pub fn cpu_policy(&self) -> &GuestCpuPolicy {
        &self.cpu_policy
    }

    pub fn create_vm(&self) -> Result<Vm, Error> {
        let raw_fd =
            sys::ioctl_with_arg(self.fd.as_raw_fd(), sys::KVM_CREATE_VM, 0).map_err(|source| {
                Error::HostEnvironment(HostEnvironmentError::VmCreation { source })
            })?;
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        sys::set_identity_map_addr(fd.as_raw_fd(), KVM_IDENTITY_MAP_ADDR).map_err(|source| {
            Error::HostEnvironment(HostEnvironmentError::VmOperation {
                operation: "KVM_SET_IDENTITY_MAP_ADDR",
                source,
            })
        })?;
        sys::set_tss_addr(fd.as_raw_fd(), KVM_TSS_ADDR).map_err(|source| {
            Error::HostEnvironment(HostEnvironmentError::VmOperation {
                operation: "KVM_SET_TSS_ADDR",
                source,
            })
        })?;

        Ok(Vm {
            fd,
            guest_memory: None,
            vcpu_mmap_size: usize::try_from(self.capabilities.vcpu_mmap_size)
                .expect("validated positive i32 always fits usize"),
            cpu_policy: self.cpu_policy.clone(),
            supports_internal_error_data: self.capabilities.supports_internal_error_data(),
        })
    }
}

#[derive(Debug)]
pub struct Vm {
    fd: OwnedFd,
    guest_memory: Option<GuestMemory>,
    vcpu_mmap_size: usize,
    cpu_policy: GuestCpuPolicy,
    supports_internal_error_data: bool,
}

impl Vm {
    pub fn register_guest_memory(&mut self, memory: GuestMemory) -> Result<(), Error> {
        if self.guest_memory.is_some() {
            return Err(Error::GuestMemory(GuestMemoryError::AlreadyRegistered));
        }

        validate_guest_memory_registration(memory.region())?;
        let region = memory.region();
        let kvm_region = sys::KvmUserspaceMemoryRegion::ram_slot0(
            region.base().get(),
            region.size(),
            memory.userspace_addr(),
        );
        sys::set_user_memory_region(self.fd.as_raw_fd(), &kvm_region)
            .map_err(|source| Error::GuestMemory(GuestMemoryError::Registration { source }))?;
        self.guest_memory = Some(memory);
        Ok(())
    }

    #[must_use]
    pub fn guest_memory(&self) -> Option<&GuestMemory> {
        self.guest_memory.as_ref()
    }

    pub fn guest_memory_mut(&mut self) -> Option<&mut GuestMemory> {
        self.guest_memory.as_mut()
    }

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
        apply_cpu_policy(&self.cpu_policy, id, &fd)?;
        let readback = read_vcpu_cpuid(id, &fd)?;
        verify_cpu_policy_readback(&self.cpu_policy, id, &readback)?;
        Vcpu::from_kvm_fd(
            id,
            fd,
            self.vcpu_mmap_size,
            self.supports_internal_error_data,
        )
    }

    fn unregister_guest_memory(&self) -> io::Result<()> {
        let region = sys::KvmUserspaceMemoryRegion::unregister_slot0();
        sys::set_user_memory_region(self.fd.as_raw_fd(), &region)
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        if self.guest_memory.is_none() {
            return;
        }

        if self.unregister_guest_memory().is_err() {
            // A vCPU fd can keep the kernel VM alive after this userspace VM handle is dropped.
            // If slot removal cannot be confirmed, leaking the mapping is safer than leaving KVM
            // with a userspace address that has already been unmapped.
            if let Some(memory) = self.guest_memory.take() {
                std::mem::forget(memory);
            }
        }
    }
}

fn check_extension(fd: &File, name: &'static str, id: i32) -> Result<Capability, Error> {
    let capability_id = libc::c_ulong::try_from(id).expect("KVM capability IDs are non-negative");
    let value = sys::ioctl_with_arg(fd.as_raw_fd(), sys::KVM_CHECK_EXTENSION, capability_id)
        .map_err(|source| host_io("KVM_CHECK_EXTENSION", source))?;
    Ok(Capability { name, id, value })
}

fn query_host_cpuid(fd: &File) -> Result<HostCpuid, Error> {
    let mut buffer = sys::KvmCpuid2::<KVM_MAX_SUPPORTED_CPUID_ENTRIES>::new();
    sys::get_supported_cpuid(fd.as_raw_fd(), &mut buffer)
        .map_err(|source| host_io("KVM_GET_SUPPORTED_CPUID", source))?;

    let count = validate_supported_cpuid_count(buffer.nent, KVM_MAX_SUPPORTED_CPUID_ENTRIES)?;
    let entries = buffer.entries[..count]
        .iter()
        .copied()
        .map(cpuid_entry_from_kvm)
        .collect();
    Ok(HostCpuid::from_entries(entries))
}

fn query_host_msr_indices(fd: &File) -> Result<HostMsrIndexList, Error> {
    let mut probe = sys::KvmMsrList::<0>::new();
    match sys::get_msr_index_list(fd.as_raw_fd(), &mut probe) {
        Err(source) if source.raw_os_error() == Some(libc::E2BIG) => {}
        Err(source) => return Err(host_io("KVM_GET_MSR_INDEX_LIST probe", source)),
        Ok(()) => {}
    }

    validate_msr_index_count(probe.nmsrs, KVM_MAX_MSR_INDEX_LIST_ENTRIES)?;

    let mut buffer = sys::KvmMsrList::<KVM_MAX_MSR_INDEX_LIST_ENTRIES>::new();
    sys::get_msr_index_list(fd.as_raw_fd(), &mut buffer)
        .map_err(|source| host_io("KVM_GET_MSR_INDEX_LIST", source))?;
    let count = validate_msr_index_count(buffer.nmsrs, KVM_MAX_MSR_INDEX_LIST_ENTRIES)?;
    Ok(HostMsrIndexList::from_validated_raw(
        &buffer.indices[..count],
    ))
}

fn query_host_msr_feature_indices(fd: &File) -> Result<HostMsrFeatureIndexList, Error> {
    let mut probe = sys::KvmMsrList::<0>::new();
    match sys::get_msr_feature_index_list(fd.as_raw_fd(), &mut probe) {
        Err(source) if source.raw_os_error() == Some(libc::E2BIG) => {}
        Err(source) => return Err(host_io("KVM_GET_MSR_FEATURE_INDEX_LIST probe", source)),
        Ok(()) => {}
    }

    validate_msr_feature_index_count(probe.nmsrs, KVM_MAX_MSR_INDEX_LIST_ENTRIES)?;

    let mut buffer = sys::KvmMsrList::<KVM_MAX_MSR_INDEX_LIST_ENTRIES>::new();
    sys::get_msr_feature_index_list(fd.as_raw_fd(), &mut buffer)
        .map_err(|source| host_io("KVM_GET_MSR_FEATURE_INDEX_LIST", source))?;
    let count = validate_msr_feature_index_count(buffer.nmsrs, KVM_MAX_MSR_INDEX_LIST_ENTRIES)?;
    Ok(HostMsrFeatureIndexList::from_validated_raw(
        &buffer.indices[..count],
    ))
}

fn query_host_msr_feature_values(
    fd: &File,
    indices: &HostMsrFeatureIndexList,
) -> Result<HostMsrFeatureValues, Error> {
    let expected = indices.indices();
    if expected.is_empty() {
        return Ok(HostMsrFeatureValues::from_values(Vec::new()));
    }

    debug_assert!(expected.len() <= KVM_MAX_MSR_INDEX_LIST_ENTRIES);
    let mut buffer = sys::KvmMsrs::<KVM_MAX_MSR_INDEX_LIST_ENTRIES>::new();
    buffer.nmsrs = u32::try_from(expected.len()).expect("validated MSR feature count fits u32");
    for (entry, index) in buffer.entries[..expected.len()]
        .iter_mut()
        .zip(expected.iter().copied())
    {
        entry.index = index.get();
    }

    let returned = sys::get_msrs(fd.as_raw_fd(), &mut buffer)
        .map_err(|source| host_io("KVM_GET_MSRS feature values", source))?;
    decode_host_msr_feature_values(expected, returned, &buffer.entries[..expected.len()])
}

fn decode_host_msr_feature_values(
    expected: &[MsrIndex],
    returned: usize,
    entries: &[sys::KvmMsrEntry],
) -> Result<HostMsrFeatureValues, Error> {
    if returned != expected.len() {
        let detail = if returned < expected.len() {
            format!(
                "KVM_GET_MSRS returned {returned} of {} requested feature MSRs; first unread index {:#x}",
                expected.len(),
                expected[returned].get()
            )
        } else {
            format!(
                "KVM_GET_MSRS returned {returned} feature MSRs after {} were requested",
                expected.len()
            )
        };
        return Err(host_io(
            "validate KVM_GET_MSRS feature response",
            io::Error::new(io::ErrorKind::InvalidData, detail),
        ));
    }

    if entries.len() < expected.len() {
        return Err(host_io(
            "validate KVM_GET_MSRS feature response",
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "KVM_GET_MSRS response buffer has {} entries for {} requested feature MSRs",
                    entries.len(),
                    expected.len()
                ),
            ),
        ));
    }

    let mut values = Vec::with_capacity(expected.len());
    for (position, (expected_index, entry)) in expected
        .iter()
        .copied()
        .zip(entries.iter().copied())
        .enumerate()
    {
        if entry.index != expected_index.get() {
            return Err(host_io(
                "validate KVM_GET_MSRS feature response",
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "KVM_GET_MSRS changed feature index at entry {position}: expected {:#x}, got {:#x}",
                        expected_index.get(),
                        entry.index
                    ),
                ),
            ));
        }
        values.push(MsrFeatureValue::new(expected_index, entry.data));
    }

    Ok(HostMsrFeatureValues::from_values(values))
}

fn apply_cpu_policy(policy: &GuestCpuPolicy, id: VcpuId, fd: &OwnedFd) -> Result<(), Error> {
    let mut buffer = sys::KvmCpuid2::<KVM_MAX_SUPPORTED_CPUID_ENTRIES>::new();
    let count = policy.entries().len();
    debug_assert!(count <= KVM_MAX_SUPPORTED_CPUID_ENTRIES);
    buffer.nent = u32::try_from(count).expect("validated KVM CPUID count fits u32");
    for (destination, source) in buffer.entries[..count]
        .iter_mut()
        .zip(policy.entries().iter().copied())
    {
        *destination = cpuid_entry_to_kvm(source);
    }

    sys::set_cpuid2(fd.as_raw_fd(), &buffer).map_err(|source| {
        Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
            id: id.get(),
            operation: "KVM_SET_CPUID2",
            source,
        })
    })
}

fn read_vcpu_cpuid(id: VcpuId, fd: &OwnedFd) -> Result<Vec<CpuidEntry>, Error> {
    let mut buffer = sys::KvmCpuid2::<KVM_MAX_SUPPORTED_CPUID_ENTRIES>::new();
    sys::get_cpuid2(fd.as_raw_fd(), &mut buffer).map_err(|source| {
        Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
            id: id.get(),
            operation: "KVM_GET_CPUID2",
            source,
        })
    })?;

    let count = validate_vcpu_cpuid_count(id, buffer.nent, KVM_MAX_SUPPORTED_CPUID_ENTRIES)?;
    Ok(buffer.entries[..count]
        .iter()
        .copied()
        .map(cpuid_entry_from_kvm)
        .collect())
}

fn verify_cpu_policy_readback(
    policy: &GuestCpuPolicy,
    id: VcpuId,
    actual: &[CpuidEntry],
) -> Result<(), Error> {
    let expected = policy.entries();
    if expected == actual {
        return Ok(());
    }

    let first_mismatch = expected
        .iter()
        .zip(actual)
        .position(|(expected, actual)| expected != actual);
    let detail = match first_mismatch {
        Some(index) => format!(
            "KVM CPUID read-back differs at entry {index}: expected {:?}, got {:?}",
            expected[index], actual[index]
        ),
        None => format!(
            "KVM CPUID read-back entry count differs: expected {}, got {}",
            expected.len(),
            actual.len()
        ),
    };

    Err(Error::HostEnvironment(
        HostEnvironmentError::VcpuOperation {
            id: id.get(),
            operation: "verify KVM_GET_CPUID2 policy",
            source: io::Error::new(io::ErrorKind::InvalidData, detail),
        },
    ))
}

fn cpuid_entry_from_kvm(entry: sys::KvmCpuidEntry2) -> CpuidEntry {
    CpuidEntry {
        function: entry.function,
        index: entry.index,
        flags: entry.flags,
        eax: entry.eax,
        ebx: entry.ebx,
        ecx: entry.ecx,
        edx: entry.edx,
    }
}

fn cpuid_entry_to_kvm(entry: CpuidEntry) -> sys::KvmCpuidEntry2 {
    sys::KvmCpuidEntry2 {
        function: entry.function,
        index: entry.index,
        flags: entry.flags,
        eax: entry.eax,
        ebx: entry.ebx,
        ecx: entry.ecx,
        edx: entry.edx,
        padding: [0; 3],
    }
}

fn validate_supported_cpuid_count(reported: u32, capacity: usize) -> Result<usize, Error> {
    let count =
        usize::try_from(reported).map_err(|_| malformed_supported_cpuid(reported, capacity))?;
    if count == 0 || count > capacity {
        return Err(malformed_supported_cpuid(reported, capacity));
    }
    Ok(count)
}

fn malformed_supported_cpuid(reported: u32, capacity: usize) -> Error {
    host_io(
        "validate KVM_GET_SUPPORTED_CPUID response",
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("KVM reported {reported} supported CPUID entries; expected 1..={capacity}"),
        ),
    )
}

fn validate_msr_index_count(reported: u32, capacity: usize) -> Result<usize, Error> {
    let count =
        usize::try_from(reported).map_err(|_| malformed_msr_index_count(reported, capacity))?;
    if count == 0 || count > capacity {
        return Err(malformed_msr_index_count(reported, capacity));
    }
    Ok(count)
}

fn malformed_msr_index_count(reported: u32, capacity: usize) -> Error {
    host_io(
        "validate KVM_GET_MSR_INDEX_LIST response",
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("KVM reported {reported} MSR indices; expected 1..={capacity}"),
        ),
    )
}

fn validate_msr_feature_index_count(reported: u32, capacity: usize) -> Result<usize, Error> {
    let count = usize::try_from(reported)
        .map_err(|_| malformed_msr_feature_index_count(reported, capacity))?;
    if count > capacity {
        return Err(malformed_msr_feature_index_count(reported, capacity));
    }
    Ok(count)
}

fn malformed_msr_feature_index_count(reported: u32, capacity: usize) -> Error {
    host_io(
        "validate KVM_GET_MSR_FEATURE_INDEX_LIST response",
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("KVM reported {reported} MSR feature indices; expected 0..={capacity}"),
        ),
    )
}

fn validate_vcpu_cpuid_count(id: VcpuId, reported: u32, capacity: usize) -> Result<usize, Error> {
    let invalid = || malformed_vcpu_cpuid(id, reported, capacity);
    let count = usize::try_from(reported).map_err(|_| invalid())?;
    if count == 0 || count > capacity {
        return Err(invalid());
    }
    Ok(count)
}

fn malformed_vcpu_cpuid(id: VcpuId, reported: u32, capacity: usize) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: id.get(),
        operation: "validate KVM_GET_CPUID2 response",
        source: io::Error::new(
            io::ErrorKind::InvalidData,
            format!("KVM reported {reported} vCPU CPUID entries; expected 1..={capacity}"),
        ),
    })
}

fn reserved_kvm_x86_region() -> GuestMemoryRegion {
    GuestMemoryRegion::new(
        GuestPhysAddr::new(KVM_IDENTITY_MAP_ADDR),
        KVM_RESERVED_X86_SIZE,
    )
    .expect("KVM reserved x86 range constants are page aligned and non-overflowing")
}

fn validate_guest_memory_registration(region: GuestMemoryRegion) -> Result<(), Error> {
    let reserved = reserved_kvm_x86_region();
    if region.overlaps(reserved) {
        return Err(Error::GuestMemory(GuestMemoryError::ReservedRangeOverlap {
            region_base: region.base().get(),
            region_size: region.size(),
            reserved_base: reserved.base().get(),
            reserved_size: reserved.size(),
        }));
    }
    Ok(())
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
            extensions: REQUIRED_EXTENSIONS
                .into_iter()
                .map(|(name, id)| Capability { name, id, value: 1 })
                .collect(),
        }
    }

    fn policy_fixture() -> GuestCpuPolicy {
        GuestCpuPolicy::from_host(&HostCpuid::from_entries(vec![CpuidEntry {
            function: 0x8000_0001,
            index: 2,
            flags: 0x55aa_aa55,
            eax: 0x1111_1111,
            ebx: 0x2222_2222,
            ecx: 0x3333_3333,
            edx: 0x4444_4444,
        }]))
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
        capabilities
            .extensions
            .retain(|capability| capability.id != KVM_CAP_SET_TSS_ADDR);
        assert!(matches!(
            capabilities.validate(),
            Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
                name: "KVM_CAP_SET_TSS_ADDR",
                id: KVM_CAP_SET_TSS_ADDR,
            }))
        ));
    }

    #[test]
    fn rejects_missing_extended_cpuid_support() {
        let mut capabilities = valid_capabilities();
        capabilities
            .extensions
            .retain(|capability| capability.id != KVM_CAP_EXT_CPUID);
        assert!(matches!(
            capabilities.validate(),
            Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
                name: "KVM_CAP_EXT_CPUID",
                id: KVM_CAP_EXT_CPUID,
            }))
        ));
    }

    #[test]
    fn rejects_missing_msr_feature_support() {
        let mut capabilities = valid_capabilities();
        capabilities
            .extensions
            .retain(|capability| capability.id != KVM_CAP_GET_MSR_FEATURES);
        assert!(matches!(
            capabilities.validate(),
            Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
                name: "KVM_CAP_GET_MSR_FEATURES",
                id: KVM_CAP_GET_MSR_FEATURES,
            }))
        ));
    }

    #[test]
    fn rejects_disabled_required_extension() {
        let mut capabilities = valid_capabilities();
        capabilities
            .extensions
            .iter_mut()
            .find(|capability| capability.id == KVM_CAP_SET_IDENTITY_MAP_ADDR)
            .unwrap()
            .value = 0;
        assert!(matches!(
            capabilities.validate(),
            Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
                name: "KVM_CAP_SET_IDENTITY_MAP_ADDR",
                id: KVM_CAP_SET_IDENTITY_MAP_ADDR,
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

    #[test]
    fn validates_supported_cpuid_count_against_fixed_capacity() {
        assert_eq!(validate_supported_cpuid_count(1, 256).unwrap(), 1);
        assert_eq!(validate_supported_cpuid_count(256, 256).unwrap(), 256);
        assert!(matches!(
            validate_supported_cpuid_count(0, 256),
            Err(Error::HostEnvironment(HostEnvironmentError::Io {
                operation: "validate KVM_GET_SUPPORTED_CPUID response",
                ..
            }))
        ));
        assert!(matches!(
            validate_supported_cpuid_count(257, 256),
            Err(Error::HostEnvironment(HostEnvironmentError::Io {
                operation: "validate KVM_GET_SUPPORTED_CPUID response",
                ..
            }))
        ));
    }

    #[test]
    fn validates_msr_index_count_against_project_bound() {
        assert_eq!(validate_msr_index_count(1, 1024).unwrap(), 1);
        assert_eq!(validate_msr_index_count(1024, 1024).unwrap(), 1024);
        assert!(matches!(
            validate_msr_index_count(0, 1024),
            Err(Error::HostEnvironment(HostEnvironmentError::Io {
                operation: "validate KVM_GET_MSR_INDEX_LIST response",
                ..
            }))
        ));
        assert!(matches!(
            validate_msr_index_count(1025, 1024),
            Err(Error::HostEnvironment(HostEnvironmentError::Io {
                operation: "validate KVM_GET_MSR_INDEX_LIST response",
                ..
            }))
        ));
    }

    #[test]
    fn validates_msr_feature_index_count_against_project_bound() {
        assert_eq!(validate_msr_feature_index_count(0, 1024).unwrap(), 0);
        assert_eq!(validate_msr_feature_index_count(1, 1024).unwrap(), 1);
        assert_eq!(validate_msr_feature_index_count(1024, 1024).unwrap(), 1024);
        assert!(matches!(
            validate_msr_feature_index_count(1025, 1024),
            Err(Error::HostEnvironment(HostEnvironmentError::Io {
                operation: "validate KVM_GET_MSR_FEATURE_INDEX_LIST response",
                ..
            }))
        ));
    }

    #[test]
    fn exact_msr_feature_value_response_is_accepted() {
        let expected = [MsrIndex::new(0x3a), MsrIndex::new(0x10a)];
        let entries = [
            sys::KvmMsrEntry {
                index: 0x3a,
                reserved: 0,
                data: 0x1111_2222_3333_4444,
            },
            sys::KvmMsrEntry {
                index: 0x10a,
                reserved: 0,
                data: 0xaaaa_bbbb_cccc_dddd,
            },
        ];
        let snapshot = decode_host_msr_feature_values(&expected, 2, &entries).unwrap();
        assert_eq!(snapshot.values().len(), 2);
        assert_eq!(snapshot.values()[0].index(), expected[0]);
        assert_eq!(snapshot.values()[0].value(), 0x1111_2222_3333_4444);
        assert_eq!(snapshot.values()[1].index(), expected[1]);
        assert_eq!(snapshot.values()[1].value(), 0xaaaa_bbbb_cccc_dddd);
    }

    #[test]
    fn partial_msr_feature_value_response_is_rejected_with_first_unread_index() {
        let expected = [MsrIndex::new(0x3a), MsrIndex::new(0x10a)];
        let entries = [
            sys::KvmMsrEntry {
                index: 0x3a,
                reserved: 0,
                data: 1,
            },
            sys::KvmMsrEntry {
                index: 0x10a,
                reserved: 0,
                data: 0,
            },
        ];
        let error = decode_host_msr_feature_values(&expected, 1, &entries).unwrap_err();
        match error {
            Error::HostEnvironment(HostEnvironmentError::Io { operation, source }) => {
                assert_eq!(operation, "validate KVM_GET_MSRS feature response");
                assert!(source.to_string().contains("returned 1 of 2"));
                assert!(source.to_string().contains("0x10a"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn msr_feature_value_response_rejects_index_drift() {
        let expected = [MsrIndex::new(0x3a)];
        let entries = [sys::KvmMsrEntry {
            index: 0x48,
            reserved: 0,
            data: 7,
        }];
        let error = decode_host_msr_feature_values(&expected, 1, &entries).unwrap_err();
        match error {
            Error::HostEnvironment(HostEnvironmentError::Io { operation, source }) => {
                assert_eq!(operation, "validate KVM_GET_MSRS feature response");
                assert!(source.to_string().contains("expected 0x3a, got 0x48"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn empty_msr_feature_value_response_is_valid() {
        let snapshot = decode_host_msr_feature_values(&[], 0, &[]).unwrap();
        assert!(snapshot.values().is_empty());
    }

    #[test]
    fn validates_vcpu_cpuid_readback_count_against_fixed_capacity() {
        assert_eq!(validate_vcpu_cpuid_count(VcpuId::BOOT, 1, 256).unwrap(), 1);
        assert_eq!(
            validate_vcpu_cpuid_count(VcpuId::BOOT, 256, 256).unwrap(),
            256
        );
        assert!(matches!(
            validate_vcpu_cpuid_count(VcpuId::BOOT, 0, 256),
            Err(Error::HostEnvironment(
                HostEnvironmentError::VcpuOperation {
                    id: 0,
                    operation: "validate KVM_GET_CPUID2 response",
                    ..
                }
            ))
        ));
        assert!(matches!(
            validate_vcpu_cpuid_count(VcpuId::BOOT, 257, 256),
            Err(Error::HostEnvironment(
                HostEnvironmentError::VcpuOperation {
                    id: 0,
                    operation: "validate KVM_GET_CPUID2 response",
                    ..
                }
            ))
        ));
    }

    #[test]
    fn exact_cpuid_policy_readback_is_accepted() {
        let policy = policy_fixture();
        assert!(verify_cpu_policy_readback(&policy, VcpuId::BOOT, policy.entries()).is_ok());
    }

    #[test]
    fn cpuid_policy_readback_rejects_entry_count_mismatch() {
        let policy = policy_fixture();
        assert!(matches!(
            verify_cpu_policy_readback(&policy, VcpuId::BOOT, &[]),
            Err(Error::HostEnvironment(
                HostEnvironmentError::VcpuOperation {
                    id: 0,
                    operation: "verify KVM_GET_CPUID2 policy",
                    ..
                }
            ))
        ));
    }

    #[test]
    fn cpuid_policy_readback_rejects_field_mismatch() {
        let policy = policy_fixture();
        let mut actual = policy.entries().to_vec();
        actual[0].ecx ^= 1;
        assert!(matches!(
            verify_cpu_policy_readback(&policy, VcpuId::BOOT, &actual),
            Err(Error::HostEnvironment(
                HostEnvironmentError::VcpuOperation {
                    id: 0,
                    operation: "verify KVM_GET_CPUID2 policy",
                    ..
                }
            ))
        ));
    }

    #[test]
    fn cpuid_uapi_conversion_drops_reserved_padding_and_round_trips_fields() {
        let raw = sys::KvmCpuidEntry2 {
            function: 0x8000_0001,
            index: 7,
            flags: 0xa5a5_5a5a,
            eax: 0x1111_1111,
            ebx: 0x2222_2222,
            ecx: 0x3333_3333,
            edx: 0x4444_4444,
            padding: [1, 2, 3],
        };

        let typed = cpuid_entry_from_kvm(raw);
        assert_eq!(typed.function, raw.function);
        assert_eq!(typed.index, raw.index);
        assert_eq!(typed.flags, raw.flags);
        assert_eq!(typed.eax, raw.eax);
        assert_eq!(typed.ebx, raw.ebx);
        assert_eq!(typed.ecx, raw.ecx);
        assert_eq!(typed.edx, raw.edx);

        let round_trip = cpuid_entry_to_kvm(typed);
        assert_eq!(round_trip.padding, [0; 3]);
        assert_eq!(round_trip.function, raw.function);
        assert_eq!(round_trip.index, raw.index);
        assert_eq!(round_trip.flags, raw.flags);
        assert_eq!(round_trip.eax, raw.eax);
        assert_eq!(round_trip.ebx, raw.ebx);
        assert_eq!(round_trip.ecx, raw.ecx);
        assert_eq!(round_trip.edx, raw.edx);
    }

    #[test]
    fn rejects_ram_overlapping_kvm_x86_reserved_pages() {
        let region = GuestMemoryRegion::new(
            GuestPhysAddr::new(KVM_IDENTITY_MAP_ADDR),
            KVM_MEMORY_ALIGNMENT,
        )
        .unwrap();
        assert!(matches!(
            validate_guest_memory_registration(region),
            Err(Error::GuestMemory(
                GuestMemoryError::ReservedRangeOverlap { .. }
            ))
        ));
    }

    #[test]
    fn accepts_ram_adjacent_to_kvm_x86_reserved_pages() {
        let region = GuestMemoryRegion::new(
            GuestPhysAddr::new(KVM_IDENTITY_MAP_ADDR - KVM_MEMORY_ALIGNMENT),
            KVM_MEMORY_ALIGNMENT,
        )
        .unwrap();
        assert!(validate_guest_memory_registration(region).is_ok());
    }
}
