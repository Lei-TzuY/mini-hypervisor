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

impl CpuidEntry {
    #[must_use]
    pub const fn key(self) -> CpuidPolicyKey {
        CpuidPolicyKey::new(self.function, self.index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CpuidPolicyKey {
    function: u32,
    index: u32,
}

impl CpuidPolicyKey {
    #[must_use]
    pub const fn new(function: u32, index: u32) -> Self {
        Self { function, index }
    }

    #[must_use]
    pub const fn function(self) -> u32 {
        self.function
    }

    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuidPolicyField {
    Flags,
    Eax,
    Ebx,
    Ecx,
    Edx,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuidPolicyEntryMismatch {
    key: CpuidPolicyKey,
    reference_entry: CpuidEntry,
    observed_entry: CpuidEntry,
    differing_fields: Vec<CpuidPolicyField>,
}

impl CpuidPolicyEntryMismatch {
    fn between(reference_entry: CpuidEntry, observed_entry: CpuidEntry) -> Option<Self> {
        debug_assert_eq!(reference_entry.key(), observed_entry.key());

        let mut differing_fields = Vec::with_capacity(5);
        if reference_entry.flags != observed_entry.flags {
            differing_fields.push(CpuidPolicyField::Flags);
        }
        if reference_entry.eax != observed_entry.eax {
            differing_fields.push(CpuidPolicyField::Eax);
        }
        if reference_entry.ebx != observed_entry.ebx {
            differing_fields.push(CpuidPolicyField::Ebx);
        }
        if reference_entry.ecx != observed_entry.ecx {
            differing_fields.push(CpuidPolicyField::Ecx);
        }
        if reference_entry.edx != observed_entry.edx {
            differing_fields.push(CpuidPolicyField::Edx);
        }

        if differing_fields.is_empty() {
            None
        } else {
            Some(Self {
                key: reference_entry.key(),
                reference_entry,
                observed_entry,
                differing_fields,
            })
        }
    }

    #[must_use]
    pub const fn key(&self) -> CpuidPolicyKey {
        self.key
    }

    #[must_use]
    pub const fn reference_entry(&self) -> CpuidEntry {
        self.reference_entry
    }

    #[must_use]
    pub const fn observed_entry(&self) -> CpuidEntry {
        self.observed_entry
    }

    #[must_use]
    pub fn differing_fields(&self) -> &[CpuidPolicyField] {
        &self.differing_fields
    }
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

    #[must_use]
    pub fn compare(&self, observed: &Self) -> GuestCpuPolicyComparison {
        let missing_from_observed = self
            .entries
            .iter()
            .copied()
            .filter(|reference| {
                !observed
                    .entries
                    .iter()
                    .any(|candidate| candidate.key() == reference.key())
            })
            .collect();
        let extra_in_observed = observed
            .entries
            .iter()
            .copied()
            .filter(|candidate| {
                !self
                    .entries
                    .iter()
                    .any(|reference| reference.key() == candidate.key())
            })
            .collect();
        let entry_mismatches = self
            .entries
            .iter()
            .filter_map(|reference| {
                observed
                    .entries
                    .iter()
                    .find(|candidate| candidate.key() == reference.key())
                    .and_then(|candidate| CpuidPolicyEntryMismatch::between(*reference, *candidate))
            })
            .collect();

        GuestCpuPolicyComparison {
            reference: self.clone(),
            observed: observed.clone(),
            missing_from_observed,
            extra_in_observed,
            entry_mismatches,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestCpuPolicyComparison {
    reference: GuestCpuPolicy,
    observed: GuestCpuPolicy,
    missing_from_observed: Vec<CpuidEntry>,
    extra_in_observed: Vec<CpuidEntry>,
    entry_mismatches: Vec<CpuidPolicyEntryMismatch>,
}

impl GuestCpuPolicyComparison {
    #[must_use]
    pub fn reference(&self) -> &GuestCpuPolicy {
        &self.reference
    }

    #[must_use]
    pub fn observed(&self) -> &GuestCpuPolicy {
        &self.observed
    }

    #[must_use]
    pub fn missing_from_observed(&self) -> &[CpuidEntry] {
        &self.missing_from_observed
    }

    #[must_use]
    pub fn extra_in_observed(&self) -> &[CpuidEntry] {
        &self.extra_in_observed
    }

    #[must_use]
    pub fn entry_mismatches(&self) -> &[CpuidPolicyEntryMismatch] {
        &self.entry_mismatches
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.missing_from_observed.is_empty()
            && self.extra_in_observed.is_empty()
            && self.entry_mismatches.is_empty()
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

    fn policy(entries: Vec<CpuidEntry>) -> GuestCpuPolicy {
        GuestCpuPolicy { entries }
    }

    fn entry(function: u32, index: u32, value: u32) -> CpuidEntry {
        CpuidEntry {
            function,
            index,
            flags: value,
            eax: value.wrapping_add(1),
            ebx: value.wrapping_add(2),
            ecx: value.wrapping_add(3),
            edx: value.wrapping_add(4),
        }
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

    #[test]
    fn cpuid_policy_key_uses_function_and_index() {
        let entry = entry(0x8000_0001, 7, 1);
        assert_eq!(entry.key(), CpuidPolicyKey::new(0x8000_0001, 7));
        assert_eq!(entry.key().function(), 0x8000_0001);
        assert_eq!(entry.key().index(), 7);
    }

    #[test]
    fn identical_guest_cpu_policies_compare_as_exact_match() {
        let reference = policy(vec![entry(1, 0, 1), entry(7, 2, 10)]);
        let observed = reference.clone();
        let comparison = reference.compare(&observed);

        assert!(comparison.is_exact_match());
        assert!(comparison.missing_from_observed().is_empty());
        assert!(comparison.extra_in_observed().is_empty());
        assert!(comparison.entry_mismatches().is_empty());
        assert_eq!(comparison.reference(), &reference);
        assert_eq!(comparison.observed(), &observed);
    }

    #[test]
    fn guest_cpu_policy_comparison_is_order_insensitive() {
        let first = entry(1, 0, 1);
        let second = entry(7, 2, 10);
        let reference = policy(vec![first, second]);
        let observed = policy(vec![second, first]);

        assert_ne!(reference, observed);
        assert!(reference.compare(&observed).is_exact_match());
    }

    #[test]
    fn guest_cpu_policy_comparison_reports_reference_entry_missing_from_observed() {
        let shared = entry(1, 0, 1);
        let missing = entry(7, 2, 10);
        let reference = policy(vec![shared, missing]);
        let observed = policy(vec![shared]);
        let comparison = reference.compare(&observed);

        assert!(!comparison.is_exact_match());
        assert_eq!(comparison.missing_from_observed(), &[missing]);
        assert!(comparison.extra_in_observed().is_empty());
        assert!(comparison.entry_mismatches().is_empty());
    }

    #[test]
    fn guest_cpu_policy_comparison_reports_observed_entry_extra_to_reference() {
        let shared = entry(1, 0, 1);
        let extra = entry(7, 2, 10);
        let reference = policy(vec![shared]);
        let observed = policy(vec![shared, extra]);
        let comparison = reference.compare(&observed);

        assert!(!comparison.is_exact_match());
        assert!(comparison.missing_from_observed().is_empty());
        assert_eq!(comparison.extra_in_observed(), &[extra]);
        assert!(comparison.entry_mismatches().is_empty());
    }

    #[test]
    fn same_function_with_different_index_is_a_distinct_policy_key() {
        let reference_entry = entry(7, 0, 1);
        let observed_entry = entry(7, 1, 1);
        let comparison = policy(vec![reference_entry]).compare(&policy(vec![observed_entry]));

        assert_eq!(comparison.missing_from_observed(), &[reference_entry]);
        assert_eq!(comparison.extra_in_observed(), &[observed_entry]);
        assert!(comparison.entry_mismatches().is_empty());
    }

    #[test]
    fn same_index_with_different_function_is_a_distinct_policy_key() {
        let reference_entry = entry(1, 0, 1);
        let observed_entry = entry(7, 0, 1);
        let comparison = policy(vec![reference_entry]).compare(&policy(vec![observed_entry]));

        assert_eq!(comparison.missing_from_observed(), &[reference_entry]);
        assert_eq!(comparison.extra_in_observed(), &[observed_entry]);
        assert!(comparison.entry_mismatches().is_empty());
    }

    #[test]
    fn same_key_mismatch_reports_every_changed_contract_field_in_canonical_order() {
        let reference_entry = CpuidEntry {
            function: 7,
            index: 2,
            flags: 1,
            eax: 2,
            ebx: 3,
            ecx: 4,
            edx: 5,
        };
        let observed_entry = CpuidEntry {
            function: 7,
            index: 2,
            flags: 11,
            eax: 12,
            ebx: 13,
            ecx: 14,
            edx: 15,
        };
        let comparison = policy(vec![reference_entry]).compare(&policy(vec![observed_entry]));
        let mismatch = &comparison.entry_mismatches()[0];

        assert_eq!(mismatch.key(), CpuidPolicyKey::new(7, 2));
        assert_eq!(mismatch.reference_entry(), reference_entry);
        assert_eq!(mismatch.observed_entry(), observed_entry);
        assert_eq!(
            mismatch.differing_fields(),
            &[
                CpuidPolicyField::Flags,
                CpuidPolicyField::Eax,
                CpuidPolicyField::Ebx,
                CpuidPolicyField::Ecx,
                CpuidPolicyField::Edx,
            ]
        );
        assert!(comparison.missing_from_observed().is_empty());
        assert!(comparison.extra_in_observed().is_empty());
    }

    #[test]
    fn single_same_key_field_change_is_not_reported_as_missing_or_extra() {
        let reference_entry = entry(1, 0, 1);
        let mut observed_entry = reference_entry;
        observed_entry.ecx ^= 1;
        let comparison = policy(vec![reference_entry]).compare(&policy(vec![observed_entry]));

        assert!(comparison.missing_from_observed().is_empty());
        assert!(comparison.extra_in_observed().is_empty());
        assert_eq!(comparison.entry_mismatches().len(), 1);
        assert_eq!(
            comparison.entry_mismatches()[0].differing_fields(),
            &[CpuidPolicyField::Ecx]
        );
    }

    #[test]
    fn guest_cpu_policy_comparison_preserves_directional_source_order() {
        let shared = entry(1, 0, 1);
        let missing_a = entry(7, 0, 10);
        let mismatch_reference = entry(7, 1, 20);
        let missing_b = entry(7, 2, 30);
        let mut mismatch_observed = mismatch_reference;
        mismatch_observed.eax ^= 1;
        let extra_a = entry(0x4000_0001, 0, 40);
        let extra_b = entry(0x8000_0001, 0, 50);
        let reference = policy(vec![shared, missing_a, mismatch_reference, missing_b]);
        let observed = policy(vec![extra_a, mismatch_observed, shared, extra_b]);
        let comparison = reference.compare(&observed);

        assert_eq!(comparison.missing_from_observed(), &[missing_a, missing_b]);
        assert_eq!(comparison.extra_in_observed(), &[extra_a, extra_b]);
        assert_eq!(comparison.entry_mismatches().len(), 1);
        assert_eq!(
            comparison.entry_mismatches()[0].key(),
            mismatch_reference.key()
        );
        assert_eq!(
            comparison.entry_mismatches()[0].differing_fields(),
            &[CpuidPolicyField::Eax]
        );
    }

    #[test]
    fn guest_cpu_policy_comparison_owns_complete_policy_provenance() {
        let reference = policy(vec![entry(1, 0, 1), entry(7, 0, 10)]);
        let observed = policy(vec![entry(1, 0, 1)]);
        let comparison = reference.compare(&observed);

        assert_eq!(comparison.reference(), &reference);
        assert_eq!(comparison.observed(), &observed);
        assert_eq!(comparison.reference().entries(), reference.entries());
        assert_eq!(comparison.observed().entries(), observed.entries());
    }

    #[test]
    fn empty_guest_cpu_policies_compare_as_exact_match() {
        let reference = policy(Vec::new());
        let observed = policy(Vec::new());

        assert!(reference.compare(&observed).is_exact_match());
    }
}
