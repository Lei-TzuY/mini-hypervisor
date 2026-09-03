use std::io;
use std::os::fd::RawFd;

pub const KVM_GET_API_VERSION: libc::c_ulong = 0xAE00;
pub const KVM_CREATE_VM: libc::c_ulong = 0xAE01;
pub const KVM_GET_MSR_INDEX_LIST: libc::c_ulong = 0xC004_AE02;
pub const KVM_CHECK_EXTENSION: libc::c_ulong = 0xAE03;
pub const KVM_GET_VCPU_MMAP_SIZE: libc::c_ulong = 0xAE04;
pub const KVM_GET_SUPPORTED_CPUID: libc::c_ulong = 0xC008_AE05;
pub const KVM_CREATE_VCPU: libc::c_ulong = 0xAE41;
pub const KVM_SET_USER_MEMORY_REGION: libc::c_ulong = 0x4020_AE46;
pub const KVM_SET_TSS_ADDR: libc::c_ulong = 0xAE47;
pub const KVM_SET_IDENTITY_MAP_ADDR: libc::c_ulong = 0x4008_AE48;
pub const KVM_RUN: libc::c_ulong = 0xAE80;
pub const KVM_GET_REGS: libc::c_ulong = 0x8090_AE81;
pub const KVM_SET_REGS: libc::c_ulong = 0x4090_AE82;
pub const KVM_GET_SREGS: libc::c_ulong = 0x8138_AE83;
pub const KVM_SET_SREGS: libc::c_ulong = 0x4138_AE84;
pub const KVM_SET_CPUID2: libc::c_ulong = 0x4008_AE90;
pub const KVM_GET_CPUID2: libc::c_ulong = 0xC008_AE91;
pub const KVM_EXIT_IO: u32 = 2;
pub const KVM_EXIT_HLT: u32 = 5;
pub const KVM_EXIT_IO_IN: u8 = 0;
pub const KVM_EXIT_IO_OUT: u8 = 1;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KvmUserspaceMemoryRegion {
    pub slot: u32,
    pub flags: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
}

impl KvmUserspaceMemoryRegion {
    #[must_use]
    pub const fn ram_slot0(guest_phys_addr: u64, memory_size: u64, userspace_addr: u64) -> Self {
        Self {
            slot: 0,
            flags: 0,
            guest_phys_addr,
            memory_size,
            userspace_addr,
        }
    }

    #[must_use]
    pub const fn unregister_slot0() -> Self {
        Self {
            slot: 0,
            flags: 0,
            guest_phys_addr: 0,
            memory_size: 0,
            userspace_addr: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvmRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvmSegment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
    pub unusable: u8,
    pub padding: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvmDtable {
    pub base: u64,
    pub limit: u16,
    pub padding: [u16; 3],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvmSregs {
    pub cs: KvmSegment,
    pub ds: KvmSegment,
    pub es: KvmSegment,
    pub fs: KvmSegment,
    pub gs: KvmSegment,
    pub ss: KvmSegment,
    pub tr: KvmSegment,
    pub ldt: KvmSegment,
    pub gdt: KvmDtable,
    pub idt: KvmDtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvmCpuidEntry2 {
    pub function: u32,
    pub index: u32,
    pub flags: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub padding: [u32; 3],
}

impl KvmCpuidEntry2 {
    pub const ZERO: Self = Self {
        function: 0,
        index: 0,
        flags: 0,
        eax: 0,
        ebx: 0,
        ecx: 0,
        edx: 0,
        padding: [0; 3],
    };
}

#[repr(C)]
#[derive(Debug)]
pub struct KvmCpuid2<const N: usize> {
    pub nent: u32,
    pub padding: u32,
    pub entries: [KvmCpuidEntry2; N],
}

impl<const N: usize> KvmCpuid2<N> {
    pub fn new() -> Self {
        let nent = u32::try_from(N).expect("KVM CPUID entry capacity fits in u32");
        Self {
            nent,
            padding: 0,
            entries: [KvmCpuidEntry2::ZERO; N],
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct KvmMsrList<const N: usize> {
    pub nmsrs: u32,
    pub indices: [u32; N],
}

impl<const N: usize> KvmMsrList<N> {
    pub fn new() -> Self {
        let nmsrs = u32::try_from(N).expect("KVM MSR index capacity fits in u32");
        Self {
            nmsrs,
            indices: [0; N],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvmRunHeader {
    pub request_interrupt_window: u8,
    pub immediate_exit: u8,
    pub padding1: [u8; 6],
    pub exit_reason: u32,
    pub ready_for_interrupt_injection: u8,
    pub if_flag: u8,
    pub flags: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvmRunIo {
    pub direction: u8,
    pub size: u8,
    pub port: u16,
    pub count: u32,
    pub data_offset: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvmRunIoPrefix {
    pub header: KvmRunHeader,
    pub cr8: u64,
    pub apic_base: u64,
    pub io: KvmRunIo,
}

pub fn ioctl_noarg(fd: RawFd, request: libc::c_ulong) -> io::Result<i32> {
    let result = unsafe { libc::ioctl(fd, request) };
    cvt_ioctl(result)
}

pub fn ioctl_with_arg(fd: RawFd, request: libc::c_ulong, arg: libc::c_ulong) -> io::Result<i32> {
    let result = unsafe { libc::ioctl(fd, request, arg) };
    cvt_ioctl(result)
}

pub fn get_msr_index_list<const N: usize>(fd: RawFd, list: &mut KvmMsrList<N>) -> io::Result<()> {
    // SAFETY: `list` is one contiguous repr(C) header plus N u32 indices. KVM reads `nmsrs` as
    // the caller capacity, writes the required/actual count back, and never writes more than N
    // trailing entries when the capacity is sufficient.
    let result = unsafe { libc::ioctl(fd, KVM_GET_MSR_INDEX_LIST, list) };
    cvt_ioctl(result).map(|_| ())
}

pub fn get_supported_cpuid<const N: usize>(fd: RawFd, cpuid: &mut KvmCpuid2<N>) -> io::Result<()> {
    // SAFETY: `cpuid` is one contiguous repr(C) header plus N entries. KVM uses `nent` to bound
    // writes to the trailing variable-length array, and the caller initializes it to N.
    let result = unsafe { libc::ioctl(fd, KVM_GET_SUPPORTED_CPUID, cpuid) };
    cvt_ioctl(result).map(|_| ())
}

pub fn set_cpuid2<const N: usize>(fd: RawFd, cpuid: &KvmCpuid2<N>) -> io::Result<()> {
    // SAFETY: `cpuid` is a readable repr(C) header followed by at least `nent` initialized entries.
    let result = unsafe { libc::ioctl(fd, KVM_SET_CPUID2, cpuid) };
    cvt_ioctl(result).map(|_| ())
}

pub fn get_cpuid2<const N: usize>(fd: RawFd, cpuid: &mut KvmCpuid2<N>) -> io::Result<()> {
    // SAFETY: `cpuid` is one contiguous writable repr(C) header plus N entries. The caller sets
    // `nent` to N, which bounds KVM's copy into the trailing variable-length array.
    let result = unsafe { libc::ioctl(fd, KVM_GET_CPUID2, cpuid) };
    cvt_ioctl(result).map(|_| ())
}

pub fn set_user_memory_region(fd: RawFd, region: &KvmUserspaceMemoryRegion) -> io::Result<()> {
    // SAFETY: `region` points to a correctly laid out KVM UAPI structure for the duration of the
    // ioctl. The caller retains ownership of the backing mapping after successful registration.
    let result = unsafe { libc::ioctl(fd, KVM_SET_USER_MEMORY_REGION, region) };
    cvt_ioctl(result).map(|_| ())
}

pub fn set_tss_addr(fd: RawFd, address: u64) -> io::Result<()> {
    let address = libc::c_ulong::try_from(address).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "KVM TSS address does not fit unsigned long",
        )
    })?;
    ioctl_with_arg(fd, KVM_SET_TSS_ADDR, address).map(|_| ())
}

pub fn set_identity_map_addr(fd: RawFd, address: u64) -> io::Result<()> {
    // SAFETY: `address` is a readable u64 for the duration of the x86 KVM VM ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_SET_IDENTITY_MAP_ADDR, &address) };
    cvt_ioctl(result).map(|_| ())
}

pub fn get_regs(fd: RawFd) -> io::Result<KvmRegs> {
    let mut regs = KvmRegs::default();
    // SAFETY: `regs` is a writable x86-64 KVM register structure for the duration of the ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_GET_REGS, &mut regs) };
    cvt_ioctl(result)?;
    Ok(regs)
}

pub fn set_regs(fd: RawFd, regs: &KvmRegs) -> io::Result<()> {
    // SAFETY: `regs` is a readable x86-64 KVM register structure for the duration of the ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_SET_REGS, regs) };
    cvt_ioctl(result).map(|_| ())
}

pub fn get_sregs(fd: RawFd) -> io::Result<KvmSregs> {
    let mut sregs = KvmSregs::default();
    // SAFETY: `sregs` is a writable x86-64 KVM special-register structure for the ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_GET_SREGS, &mut sregs) };
    cvt_ioctl(result)?;
    Ok(sregs)
}

pub fn set_sregs(fd: RawFd, sregs: &KvmSregs) -> io::Result<()> {
    // SAFETY: `sregs` is a readable x86-64 KVM special-register structure for the ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_SET_SREGS, sregs) };
    cvt_ioctl(result).map(|_| ())
}

pub fn run_vcpu(fd: RawFd) -> io::Result<()> {
    let result = unsafe { libc::ioctl(fd, KVM_RUN) };
    cvt_ioctl(result).map(|_| ())
}

fn cvt_ioctl(result: libc::c_int) -> io::Result<i32> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userspace_memory_region_matches_x86_64_kvm_uapi_layout() {
        assert_eq!(std::mem::size_of::<KvmUserspaceMemoryRegion>(), 32);
        assert_eq!(KVM_SET_USER_MEMORY_REGION, 0x4020_AE46);
    }

    #[test]
    fn x86_vm_setup_ioctls_match_kvm_uapi() {
        assert_eq!(KVM_SET_TSS_ADDR, 0xAE47);
        assert_eq!(KVM_SET_IDENTITY_MAP_ADDR, 0x4008_AE48);
    }

    #[test]
    fn register_structures_match_x86_64_kvm_uapi_layout() {
        assert_eq!(std::mem::size_of::<KvmRegs>(), 144);
        assert_eq!(std::mem::size_of::<KvmSegment>(), 24);
        assert_eq!(std::mem::size_of::<KvmDtable>(), 16);
        assert_eq!(std::mem::size_of::<KvmSregs>(), 312);
        assert_eq!(KVM_GET_REGS, 0x8090_AE81);
        assert_eq!(KVM_SET_REGS, 0x4090_AE82);
        assert_eq!(KVM_GET_SREGS, 0x8138_AE83);
        assert_eq!(KVM_SET_SREGS, 0x4138_AE84);
    }

    #[test]
    fn cpuid_structures_match_x86_64_kvm_uapi_layout() {
        assert_eq!(std::mem::size_of::<KvmCpuidEntry2>(), 40);
        assert_eq!(std::mem::size_of::<KvmCpuid2<0>>(), 8);
        assert_eq!(std::mem::size_of::<KvmCpuid2<1>>(), 48);
        assert_eq!(std::mem::offset_of!(KvmCpuid2<1>, entries), 8);
        assert_eq!(KVM_GET_SUPPORTED_CPUID, 0xC008_AE05);
        assert_eq!(KVM_SET_CPUID2, 0x4008_AE90);
        assert_eq!(KVM_GET_CPUID2, 0xC008_AE91);
    }

    #[test]
    fn cpuid_buffer_initializes_header_and_reserved_fields() {
        let cpuid = KvmCpuid2::<3>::new();
        assert_eq!(cpuid.nent, 3);
        assert_eq!(cpuid.padding, 0);
        assert_eq!(cpuid.entries, [KvmCpuidEntry2::ZERO; 3]);
    }

    #[test]
    fn msr_index_list_matches_x86_64_kvm_uapi_layout() {
        assert_eq!(std::mem::size_of::<KvmMsrList<0>>(), 4);
        assert_eq!(std::mem::size_of::<KvmMsrList<1>>(), 8);
        assert_eq!(std::mem::offset_of!(KvmMsrList<1>, indices), 4);
        assert_eq!(KVM_GET_MSR_INDEX_LIST, 0xC004_AE02);
    }

    #[test]
    fn msr_index_list_initializes_capacity_and_entries() {
        let list = KvmMsrList::<3>::new();
        assert_eq!(list.nmsrs, 3);
        assert_eq!(list.indices, [0; 3]);
    }

    #[test]
    fn run_header_matches_kvm_uapi_prefix() {
        assert_eq!(std::mem::size_of::<KvmRunHeader>(), 16);
        assert_eq!(std::mem::offset_of!(KvmRunHeader, exit_reason), 8);
        assert_eq!(KVM_RUN, 0xAE80);
        assert_eq!(KVM_EXIT_HLT, 5);
    }

    #[test]
    fn io_exit_matches_x86_64_kvm_run_layout() {
        assert_eq!(KVM_EXIT_IO, 2);
        assert_eq!(KVM_EXIT_IO_IN, 0);
        assert_eq!(KVM_EXIT_IO_OUT, 1);
        assert_eq!(std::mem::size_of::<KvmRunIo>(), 16);
        assert_eq!(std::mem::offset_of!(KvmRunIo, data_offset), 8);
        assert_eq!(std::mem::size_of::<KvmRunIoPrefix>(), 48);
        assert_eq!(std::mem::offset_of!(KvmRunIoPrefix, io), 32);
    }

    #[test]
    fn unregister_request_removes_slot_zero() {
        assert_eq!(
            KvmUserspaceMemoryRegion::unregister_slot0(),
            KvmUserspaceMemoryRegion {
                slot: 0,
                flags: 0,
                guest_phys_addr: 0,
                memory_size: 0,
                userspace_addr: 0,
            }
        );
    }
}
