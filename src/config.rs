use crate::error::{ConfigurationError, Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmConfig {
    vcpu_count: u16,
}

impl VmConfig {
    pub const INITIAL_SUPPORTED_VCPU_COUNT: u16 = 1;

    pub fn new(vcpu_count: u16) -> Result<Self, Error> {
        if vcpu_count != Self::INITIAL_SUPPORTED_VCPU_COUNT {
            return Err(Error::Configuration(
                ConfigurationError::UnsupportedVcpuCount {
                    requested: vcpu_count,
                    supported: Self::INITIAL_SUPPORTED_VCPU_COUNT,
                },
            ));
        }
        Ok(Self { vcpu_count })
    }

    #[must_use]
    pub const fn vcpu_count(self) -> u16 {
        self.vcpu_count
    }
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            vcpu_count: Self::INITIAL_SUPPORTED_VCPU_COUNT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_single_vcpu() {
        assert_eq!(VmConfig::new(1).unwrap().vcpu_count(), 1);
    }

    #[test]
    fn rejects_zero_vcpus() {
        assert!(matches!(
            VmConfig::new(0),
            Err(Error::Configuration(
                ConfigurationError::UnsupportedVcpuCount { requested: 0, .. }
            ))
        ));
    }

    #[test]
    fn rejects_smp_before_it_is_supported() {
        assert!(matches!(
            VmConfig::new(2),
            Err(Error::Configuration(
                ConfigurationError::UnsupportedVcpuCount { requested: 2, .. }
            ))
        ));
    }
}
