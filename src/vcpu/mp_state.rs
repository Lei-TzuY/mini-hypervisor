const KVM_GET_MP_STATE: libc::c_ulong = 0x8004_AE98;
const KVM_SET_MP_STATE: libc::c_ulong = 0x4004_AE99;

pub(crate) const KVM_MP_STATE_RUNNABLE: u32 = 0;
pub(crate) const KVM_MP_STATE_UNINITIALIZED: u32 = 1;
pub(crate) const KVM_MP_STATE_INIT_RECEIVED: u32 = 2;
#[cfg(test)]
const KVM_MP_STATE_HALTED: u32 = 3;
#[cfg(test)]
const KVM_MP_STATE_SIPI_RECEIVED: u32 = 4;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KvmMpState {
    mp_state: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VcpuMpState(u32);

impl VcpuMpState {
    const RUNNABLE: Self = Self(KVM_MP_STATE_RUNNABLE);
    const UNINITIALIZED: Self = Self(KVM_MP_STATE_UNINITIALIZED);

    const fn raw(self) -> u32 {
        self.0
    }
}

// SAFETY: `KvmRunMapping` uniquely owns one process-local MAP_SHARED `kvm_run` mapping. No pointer
// into the mapping escapes the `Vcpu` boundary, moving the mapping does not relocate the mmap, and
// every userspace mutation of mapped KVM state is reached through unique `&mut Vcpu` ownership.
// This intentionally establishes `Send` only; the raw mapping remains non-`Sync`, so one `Vcpu`
// cannot be used concurrently from multiple userspace threads through shared references.
unsafe impl Send for super::KvmRunMapping {}

impl Vcpu {
    fn multiprocessing_state(&self) -> Result<VcpuMpState, Error> {
        let mut state = KvmMpState { mp_state: 0 };
        // SAFETY: `state` is the exact fixed-size Linux `struct kvm_mp_state` payload and remains
        // writable for the duration of the vCPU ioctl.
        let result = unsafe { libc::ioctl(self.fd.as_raw_fd(), KVM_GET_MP_STATE, &mut state) };
        if result == -1 {
            return Err(vcpu_operation(
                self.id,
                "KVM_GET_MP_STATE",
                io::Error::last_os_error(),
            ));
        }
        Ok(VcpuMpState(state.mp_state))
    }

    fn set_multiprocessing_state(&mut self, state: VcpuMpState) -> Result<(), Error> {
        let request = KvmMpState {
            mp_state: state.raw(),
        };
        // SAFETY: `request` is the exact fixed-size Linux `struct kvm_mp_state` payload and remains
        // readable for the duration of the vCPU ioctl. `&mut self` preserves exclusive vCPU-state
        // mutation in userspace.
        let result = unsafe { libc::ioctl(self.fd.as_raw_fd(), KVM_SET_MP_STATE, &request) };
        if result == -1 {
            return Err(vcpu_operation(
                self.id,
                "KVM_SET_MP_STATE",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    /// Read the current Linux KVM MP state without mutating vCPU startup state.
    ///
    /// In particular, INIT/SIPI execution uses this to prove the secondary vCPU transitions from
    /// UNINITIALIZED to INIT_RECEIVED and finally RUNNABLE without a userspace RUNNABLE shortcut.
    pub(crate) fn multiprocessing_state_raw(&self) -> Result<u32, Error> {
        Ok(self.multiprocessing_state()?.raw())
    }

    pub(crate) fn ensure_runnable_mp_state(&mut self) -> Result<u32, Error> {
        let initial = self.multiprocessing_state()?;
        if initial != VcpuMpState::RUNNABLE && initial != VcpuMpState::UNINITIALIZED {
            return Err(vcpu_operation(
                self.id,
                "validate initial KVM MP state",
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "expected secondary vCPU MP state RUNNABLE or UNINITIALIZED before host startup, got {}",
                        initial.raw()
                    ),
                ),
            ));
        }

        self.set_multiprocessing_state(VcpuMpState::RUNNABLE)?;
        let observed = self.multiprocessing_state()?;
        if observed != VcpuMpState::RUNNABLE {
            return Err(vcpu_operation(
                self.id,
                "verify KVM_SET_MP_STATE RUNNABLE readback",
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "expected secondary vCPU MP state RUNNABLE after host startup, got {}",
                        observed.raw()
                    ),
                ),
            ));
        }
        Ok(observed.raw())
    }
}

const _: () = {
    assert!(std::mem::size_of::<KvmMpState>() == 4);
};

#[cfg(test)]
mod mp_state_tests {
    use super::*;

    #[test]
    fn mp_state_uapi_matches_linux_kvm() {
        assert_eq!(KVM_GET_MP_STATE, 0x8004_AE98);
        assert_eq!(KVM_SET_MP_STATE, 0x4004_AE99);
        assert_eq!(std::mem::size_of::<KvmMpState>(), 4);
    }

    #[test]
    fn x86_mp_state_values_match_linux_kvm() {
        assert_eq!(KVM_MP_STATE_RUNNABLE, 0);
        assert_eq!(KVM_MP_STATE_UNINITIALIZED, 1);
        assert_eq!(KVM_MP_STATE_INIT_RECEIVED, 2);
        assert_eq!(KVM_MP_STATE_HALTED, 3);
        assert_eq!(KVM_MP_STATE_SIPI_RECEIVED, 4);
    }
}
