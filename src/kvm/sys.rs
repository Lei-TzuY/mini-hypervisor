use std::io;
use std::os::fd::RawFd;

pub const KVM_GET_API_VERSION: libc::c_ulong = 0xAE00;
pub const KVM_CREATE_VM: libc::c_ulong = 0xAE01;
pub const KVM_CHECK_EXTENSION: libc::c_ulong = 0xAE03;
pub const KVM_GET_VCPU_MMAP_SIZE: libc::c_ulong = 0xAE04;
pub const KVM_CREATE_VCPU: libc::c_ulong = 0xAE41;

pub fn ioctl_noarg(fd: RawFd, request: libc::c_ulong) -> io::Result<i32> {
    let result = unsafe { libc::ioctl(fd, request) };
    cvt_ioctl(result)
}

pub fn ioctl_with_arg(fd: RawFd, request: libc::c_ulong, arg: libc::c_ulong) -> io::Result<i32> {
    let result = unsafe { libc::ioctl(fd, request, arg) };
    cvt_ioctl(result)
}

fn cvt_ioctl(result: libc::c_int) -> io::Result<i32> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}
