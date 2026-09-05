use super::Vcpu;
use crate::error::{Error, HostEnvironmentError};
use std::io;
use std::os::fd::AsRawFd;

const KVM_GET_MP_STATE: libc::c_ulong = 0x8004_AE98;
const KVM_SET_MP_STATE: libc::c_ulong = 0x4004_AE99;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmMpState {
    mp_state: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VcpuMpState(u32);

impl VcpuMpState {
    pub(crate) const RUNNABLE: Self = Self(0);
    pub(crate) const UNINITIALIZED: Self = Self(1);
    pub(crate) const INIT_RECEIVED: Self = Self(2);
    pub(crate) const HALTED: Self = Self(3);
    pub(crate) const SIPI_RECEIVED: Self = Self(4);

    #[must_use]
    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

impl Vcpu {
    pub(crate) fn multiprocessing_state(&self) -> Result<VcpuMpState, Error> {
        let mut state = KvmMpState { mp_state: 0 };
        // SAFETY: `state` is the exact fixed-size Linux `struct kvm_mp_state` payload and remains
        // writable for the duration of the vCPU ioctl.
        let result = unsafe { libc::ioctl(self.fd.as_raw_fd(), KVM_GET_MP_STATE, &mut state) };
        if result == -1 {
            return Err(vcpu_mp_state_error(
                self,
                "KVM_GET_MP_STATE",
                io::Error::last_os_error(),
            ));
        }
        Ok(VcpuMpState(state.mp_state))
    }

    pub(crate) fn set_multiprocessing_state(&mut self, state: VcpuMpState) -> Result<(), Error> {
        let request = KvmMpState {
            mp_state: state.raw(),
        };
        // SAFETY: `request` is the exact fixed-size Linux `struct kvm_mp_state` payload and remains
        // readable for the duration of the vCPU ioctl. `&mut self` preserves exclusive vCPU-state
        // mutation in userspace.
        let result = unsafe { libc::ioctl(self.fd.as_raw_fd(), KVM_SET_MP_STATE, &request) };
        if result == -1 {
            return Err(vcpu_mp_state_error(
                self,
                "KVM_SET_MP_STATE",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }
}

fn vcpu_mp_state_error(vcpu: &Vcpu, operation: &'static str, source: io::Error) -> Error {
    Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
        id: vcpu.id().get(),
        operation,
        source,
    })
}

const _: () = {
    assert!(std::mem::size_of::<KvmMpState>() == 4);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp_state_uapi_matches_linux_kvm() {
        assert_eq!(KVM_GET_MP_STATE, 0x8004_AE98);
        assert_eq!(KVM_SET_MP_STATE, 0x4004_AE99);
        assert_eq!(std::mem::size_of::<KvmMpState>(), 4);
    }

    #[test]
    fn x86_mp_state_values_match_linux_kvm() {
        assert_eq!(VcpuMpState::RUNNABLE.raw(), 0);
        assert_eq!(VcpuMpState::UNINITIALIZED.raw(), 1);
        assert_eq!(VcpuMpState::INIT_RECEIVED.raw(), 2);
        assert_eq!(VcpuMpState::HALTED.raw(), 3);
        assert_eq!(VcpuMpState::SIPI_RECEIVED.raw(), 4);
    }
}
