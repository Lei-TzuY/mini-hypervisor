const CPUID_FEATURES: u32 = 0x0000_0001;
const CPUID_FEATURE_X2APIC: u32 = 1 << 21;
const CPUID_FEATURE_TSC_DEADLINE: u32 = 1 << 24;
const KVM_CPUID_FEATURES: u32 = 0x4000_0001;
const KVM_FEATURE_PV_UNHALT: u32 = 1 << 7;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuidEntry {
    pub function: u32,
    pub index: u32,
    pub flags: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCpuid {
    entries: Vec<CpuidEntry>,
}

impl HostCpuid {
    pub(crate) fn from_entries(entries: Vec<CpuidEntry>) -> Self {
        Self { entries }
    }

    #[must_use]
    pub fn entries(&self) -> &[CpuidEntry] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestCpuPolicy {
    entries: Vec<CpuidEntry>,
}

impl GuestCpuPolicy {
    #[must_use]
    pub fn from_host(host: &HostCpuid) -> Self {
        let mut entries = host.entries.clone();
        for entry in &mut entries {
            match entry.function {
                CPUID_FEATURES => {
                    entry.ecx &= !(CPUID_FEATURE_X2APIC | CPUID_FEATURE_TSC_DEADLINE);
                }
                KVM_CPUID_FEATURES => {
                    entry.eax &= !KVM_FEATURE_PV_UNHALT;
                }
                _ => {}
            }
        }
        Self { entries }
    }

    #[must_use]
    pub fn entries(&self) -> &[CpuidEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_fixture() -> HostCpuid {
        HostCpuid::from_entries(vec![
            CpuidEntry {
                function: CPUID_FEATURES,
                index: 3,
                flags: 0xa5a5_5a5a,
                eax: 0x1111_1111,
                ebx: 0x2222_2222,
                ecx: CPUID_FEATURE_X2APIC | CPUID_FEATURE_TSC_DEADLINE | 0x1,
                edx: 0x3333_3333,
            },
            CpuidEntry {
                function: KVM_CPUID_FEATURES,
                index: 7,
                flags: 0x55aa_aa55,
                eax: KVM_FEATURE_PV_UNHALT | 0x1,
                ebx: 0x4444_4444,
                ecx: 0x5555_5555,
                edx: 0x6666_6666,
            },
            CpuidEntry {
                function: 0x8000_0001,
                index: 9,
                flags: 0xdead_beef,
                eax: 0x7777_7777,
                ebx: 0x8888_8888,
                ecx: 0x9999_9999,
                edx: 0xaaaa_aaaa,
            },
        ])
    }

    #[test]
    fn policy_masks_only_lapic_dependent_features_without_mutating_host() {
        let host = host_fixture();
        let original = host.clone();

        let policy = GuestCpuPolicy::from_host(&host);

        assert_eq!(host, original);
        assert_eq!(policy.entries()[0].ecx, 0x1);
        assert_eq!(policy.entries()[1].eax, 0x1);
        assert_eq!(policy.entries()[2], host.entries()[2]);
    }

    #[test]
    fn policy_preserves_unrelated_metadata_and_registers() {
        let host = host_fixture();
        let policy = GuestCpuPolicy::from_host(&host);

        assert_eq!(policy.entries().len(), host.entries().len());
        assert_eq!(policy.entries()[0].function, host.entries()[0].function);
        assert_eq!(policy.entries()[0].index, host.entries()[0].index);
        assert_eq!(policy.entries()[0].flags, host.entries()[0].flags);
        assert_eq!(policy.entries()[0].eax, host.entries()[0].eax);
        assert_eq!(policy.entries()[0].ebx, host.entries()[0].ebx);
        assert_eq!(policy.entries()[0].edx, host.entries()[0].edx);
        assert_eq!(policy.entries()[1].index, host.entries()[1].index);
        assert_eq!(policy.entries()[1].flags, host.entries()[1].flags);
        assert_eq!(policy.entries()[1].ebx, host.entries()[1].ebx);
        assert_eq!(policy.entries()[1].ecx, host.entries()[1].ecx);
        assert_eq!(policy.entries()[1].edx, host.entries()[1].edx);
    }

    #[test]
    fn policy_masks_every_matching_leaf_entry() {
        let host = HostCpuid::from_entries(vec![
            CpuidEntry {
                function: CPUID_FEATURES,
                index: 0,
                ecx: CPUID_FEATURE_X2APIC | 0x2,
                ..CpuidEntry::default()
            },
            CpuidEntry {
                function: CPUID_FEATURES,
                index: 1,
                ecx: CPUID_FEATURE_TSC_DEADLINE | 0x4,
                ..CpuidEntry::default()
            },
            CpuidEntry {
                function: KVM_CPUID_FEATURES,
                index: 0,
                eax: KVM_FEATURE_PV_UNHALT | 0x8,
                ..CpuidEntry::default()
            },
        ]);

        let policy = GuestCpuPolicy::from_host(&host);

        assert_eq!(policy.entries()[0].ecx, 0x2);
        assert_eq!(policy.entries()[1].ecx, 0x4);
        assert_eq!(policy.entries()[2].eax, 0x8);
    }
}
