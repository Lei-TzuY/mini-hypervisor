use super::{vcpu_operation, Vcpu};
use crate::error::Error;
use std::io;
use std::os::fd::AsRawFd;

pub(crate) const KVM_EXIT_DEBUG: u32 = 4;
const KVM_SET_GUEST_DEBUG: libc::c_ulong = 0x4048_AE9B;
const KVM_GUESTDBG_ENABLE: u32 = 1;
const KVM_GUESTDBG_SINGLESTEP: u32 = 1 << 1;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct KvmGuestDebugArch {
    debugreg: [u64; 8],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct KvmGuestDebug {
    control: u32,
    pad: u32,
    arch: KvmGuestDebugArch,
}

impl KvmGuestDebug {
    const fn single_step(enabled: bool) -> Self {
        Self {
            control: if enabled {
                KVM_GUESTDBG_ENABLE | KVM_GUESTDBG_SINGLESTEP
            } else {
                0
            },
            pad: 0,
            arch: KvmGuestDebugArch { debugreg: [0; 8] },
        }
    }
}

impl Vcpu {
    pub(crate) fn set_guest_single_step(&mut self, enabled: bool) -> Result<(), Error> {
        let request = KvmGuestDebug::single_step(enabled);
        // SAFETY: `request` exactly matches Linux x86 `struct kvm_guest_debug`, remains readable
        // for the duration of the ioctl, and `&mut self` serializes userspace vCPU run-control
        // mutation with `KVM_RUN` and every other state transition in this process.
        let result = unsafe { libc::ioctl(self.fd.as_raw_fd(), KVM_SET_GUEST_DEBUG, &request) };
        if result == -1 {
            return Err(vcpu_operation(
                self.id,
                if enabled {
                    "KVM_SET_GUEST_DEBUG single-step enable"
                } else {
                    "KVM_SET_GUEST_DEBUG disable"
                },
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }
}

const _: () = {
    assert!(std::mem::size_of::<KvmGuestDebugArch>() == 64);
    assert!(std::mem::size_of::<KvmGuestDebug>() == 72);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_debug_uapi_matches_linux_kvm() {
        assert_eq!(KVM_EXIT_DEBUG, 4);
        assert_eq!(KVM_SET_GUEST_DEBUG, 0x4048_AE9B);
        assert_eq!(KVM_GUESTDBG_ENABLE, 1);
        assert_eq!(KVM_GUESTDBG_SINGLESTEP, 2);
        assert_eq!(std::mem::size_of::<KvmGuestDebugArch>(), 64);
        assert_eq!(std::mem::size_of::<KvmGuestDebug>(), 72);
    }

    #[test]
    fn single_step_request_changes_only_control_bits() {
        let enabled = KvmGuestDebug::single_step(true);
        assert_eq!(
            enabled.control,
            KVM_GUESTDBG_ENABLE | KVM_GUESTDBG_SINGLESTEP
        );
        assert_eq!(enabled.pad, 0);
        assert_eq!(enabled.arch.debugreg, [0; 8]);

        assert_eq!(KvmGuestDebug::single_step(false), KvmGuestDebug::default());
    }
}
