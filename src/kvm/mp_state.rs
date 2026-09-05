const KVM_CAP_MP_STATE: i32 = 14;

impl KvmBackend {
    pub(crate) fn require_mp_state_capability(&self) -> Result<(), Error> {
        let capability = super::check_extension(&self.fd, "KVM_CAP_MP_STATE", KVM_CAP_MP_STATE)?;
        if capability.value <= 0 {
            return Err(Error::KvmCapability(KvmCapabilityError::MissingExtension {
                name: "KVM_CAP_MP_STATE",
                id: KVM_CAP_MP_STATE,
            }));
        }
        Ok(())
    }
}

#[cfg(test)]
mod mp_state_capability_tests {
    use super::*;

    #[test]
    fn mp_state_capability_id_matches_linux_kvm() {
        assert_eq!(KVM_CAP_MP_STATE, 14);
    }
}
