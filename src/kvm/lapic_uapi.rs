pub const KVM_GET_LAPIC: libc::c_ulong = 0x8400_AE8E;
pub const KVM_SET_LAPIC: libc::c_ulong = 0x4400_AE8F;
pub const KVM_APIC_REG_SIZE: usize = 0x400;

#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KvmLapicState {
    pub regs: [u8; KVM_APIC_REG_SIZE],
}

impl Default for KvmLapicState {
    fn default() -> Self {
        Self {
            regs: [0; KVM_APIC_REG_SIZE],
        }
    }
}

pub fn get_lapic(fd: std::os::fd::RawFd) -> std::io::Result<KvmLapicState> {
    let mut state = KvmLapicState::default();
    // SAFETY: `state` is exactly the fixed 0x400-byte x86 `kvm_lapic_state` payload and remains
    // writable for the duration of the vCPU ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_GET_LAPIC, &mut state) };
    cvt_ioctl(result)?;
    Ok(state)
}

pub fn set_lapic(fd: std::os::fd::RawFd, state: &KvmLapicState) -> std::io::Result<()> {
    // SAFETY: `state` is exactly the fixed 0x400-byte x86 `kvm_lapic_state` payload and remains
    // readable for the duration of the vCPU ioctl.
    let result = unsafe { libc::ioctl(fd, KVM_SET_LAPIC, state) };
    cvt_ioctl(result).map(|_| ())
}

#[cfg(test)]
mod lapic_uapi_tests {
    use super::*;

    #[test]
    fn lapic_state_and_ioctls_match_x86_kvm_uapi() {
        assert_eq!(std::mem::size_of::<KvmLapicState>(), 0x400);
        assert_eq!(KVM_GET_LAPIC, 0x8400_AE8E);
        assert_eq!(KVM_SET_LAPIC, 0x4400_AE8F);
    }

    #[test]
    fn lapic_state_starts_fully_zeroed() {
        assert_eq!(KvmLapicState::default().regs, [0; KVM_APIC_REG_SIZE]);
    }
}

include!("msi.rs");
