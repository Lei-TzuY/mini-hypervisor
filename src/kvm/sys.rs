use std::io;
use std::os::fd::RawFd;

pub const KVM_GET_API_VERSION: libc::c_ulong = 0xAE00;
pub const KVM_CREATE_VM: libc::c_ulong = 0xAE01;
pub const KVM_CHECK_EXTENSION: libc::c_ulong = 0xAE03;
pub const KVM_GET_VCPU_MMAP_SIZE: libc::c_ulong = 0xAE04;
pub const KVM_CREATE_VCPU: libc::c_ulong = 0xAE41;
pub const KVM_SET_USER_MEMORY_REGION: libc::c_ulong = 0x4020_AE46;

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
