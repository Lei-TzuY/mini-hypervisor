impl Vcpu {
    pub(crate) fn capture_lapic_checkpoint_state(&self) -> Result<sys::KvmLapicState, Error> {
        sys::get_lapic(self.fd.as_raw_fd())
            .map_err(|source| vcpu_operation(self.id, "KVM_GET_LAPIC checkpoint capture", source))
    }

    pub(crate) fn restore_lapic_checkpoint_state(
        &self,
        state: &sys::KvmLapicState,
    ) -> Result<(), Error> {
        sys::set_lapic(self.fd.as_raw_fd(), state)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_LAPIC checkpoint restore", source))
    }
}

#[cfg(test)]
mod lapic_checkpoint_tests {
    use super::*;

    #[test]
    fn checkpoint_lapic_state_is_the_full_linux_kvm_payload() {
        assert_eq!(std::mem::size_of::<sys::KvmLapicState>(), 0x400);
        assert_eq!(sys::KVM_APIC_REG_SIZE, 0x400);
    }
}
