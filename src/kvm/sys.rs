use std::io;
use std::os::fd::RawFd;

pub const KVM_GET_API_VERSION: libc::c_ulong = 0xAE00;
pub const KVM_CREATE_VM: libc::c_ulong = 0xAE01;
pub const KVM_CHECK_EXTENSION: libc::c_ulong = 0xAE03;
pub const KVM_GET_VCPU_MMAP_SIZE: libc::c_ulong = 0xAE04;
pub const KVM_CREATE_VCPU: libc::c_ulong = 0xAE41;
pub const KVM_SET_USER_MEMORY_REGION: libc::c_ulong = 0x4020_AE46;
pub const KVM_SET_TSS_ADDR: libc::c_ulong = 0xAE47;
pub const KVM_SET_IDENTITY_MAP_ADDR: libc::c_ulong = 0x4008_AE48;
pub const KVM_RUN: libc::c_ulong = 0xAE80;
pub const KVM_GET_REGS: libc::c_ulong = 0x8090_AE81;
pub const KVM_SET_REGS: libc::c_ulong = 0x4090_AE82;
pub const KVM_GET_SREGS: libc::c_ulong = 0x8138_AE83;
pub const KVM_SET_SREGS: libc::c_ulong = 0x4138_AE84;
pub const KVM_EXIT_HLT: u32 = 5;

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
pub struct KvmRunHeader {
    pub request_interrupt_window: u8,
    pub immediate_exit: u8,
    pub padding1: [u8; 6],
    pub exit_reason: u32,
    pub ready_for_interrupt_injection: u8,
    pub if_flag: u8,
    pub flags: u16,
}

pub fn ioctl_noarg(fd: RawFd, request: libc::c_ulong) -> io::Result<i32> {
    let result = unsafe { libc::ioctl(fd, request) };
    cvt_ioctl(result)
}

pub fn ioctl_with_arg(fd: RawFd, request: libc::c_ulong, arg: libc::c_ulong) -> io::Result<i32> {
    let result = unsafe { libc::ioctl(fd, request, arg) };
    cvt_ioctl(result)
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
    fn run_header_matches_kvm_uapi_prefix() {
        assert_eq!(std::mem::size_of::<KvmRunHeader>(), 16);
        assert_eq!(std::mem::offset_of!(KvmRunHeader, exit_reason), 8);
        assert_eq!(KVM_RUN, 0xAE80);
        assert_eq!(KVM_EXIT_HLT, 5);
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
