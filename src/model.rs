use crate::kvm::cpu::{GuestCpuPolicy, GuestCpuPolicyComparison};
use crate::kvm::msr::{HostMsrModelCandidate, HostMsrModelComparison};
use crate::kvm::KvmBackend;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuModelCandidate {
    guest_cpu_policy: GuestCpuPolicy,
    host_msr_model_candidate: HostMsrModelCandidate,
}

impl CpuModelCandidate {
    #[must_use]
    pub fn new(
        guest_cpu_policy: &GuestCpuPolicy,
        host_msr_model_candidate: &HostMsrModelCandidate,
    ) -> Self {
        Self {
            guest_cpu_policy: guest_cpu_policy.clone(),
            host_msr_model_candidate: host_msr_model_candidate.clone(),
        }
    }

    #[must_use]
    pub fn guest_cpu_policy(&self) -> &GuestCpuPolicy {
        &self.guest_cpu_policy
    }

    #[must_use]
    pub fn host_msr_model_candidate(&self) -> &HostMsrModelCandidate {
        &self.host_msr_model_candidate
    }

    #[must_use]
    pub fn compare(&self, observed: &Self) -> CpuModelComparison {
        CpuModelComparison {
            guest_cpu_policy_comparison: self.guest_cpu_policy.compare(&observed.guest_cpu_policy),
            host_msr_model_comparison: self
                .host_msr_model_candidate
                .compare(&observed.host_msr_model_candidate),
        }
    }
}

impl KvmBackend {
    #[must_use]
    pub fn cpu_model_candidate(&self) -> CpuModelCandidate {
        let host_msr_model_candidate = self.host_msr_feature_values().model_candidate();
        CpuModelCandidate::new(self.cpu_policy(), &host_msr_model_candidate)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuModelComparison {
    guest_cpu_policy_comparison: GuestCpuPolicyComparison,
    host_msr_model_comparison: HostMsrModelComparison,
}

impl CpuModelComparison {
    #[must_use]
    pub fn guest_cpu_policy_comparison(&self) -> &GuestCpuPolicyComparison {
        &self.guest_cpu_policy_comparison
    }

    #[must_use]
    pub fn host_msr_model_comparison(&self) -> &HostMsrModelComparison {
        &self.host_msr_model_comparison
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.guest_cpu_policy_comparison.is_exact_match()
            && self.host_msr_model_comparison.is_exact_match()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kvm::cpu::{CpuidEntry, GuestCpuPolicy, HostCpuid};
    use crate::kvm::msr::{
        HostMsrFeatureValues, HostMsrModelCandidate, MsrFeatureValue, MsrIndex, MSR_IA32_UCODE_REV,
    };

    fn guest_policy(entries: Vec<CpuidEntry>) -> GuestCpuPolicy {
        GuestCpuPolicy::from_host(&HostCpuid::from_entries(entries))
    }

    fn msr_candidate(values: Vec<MsrFeatureValue>) -> HostMsrModelCandidate {
        HostMsrFeatureValues::from_values(values).model_candidate()
    }

    fn cpuid_entry(function: u32, index: u32, eax: u32) -> CpuidEntry {
        CpuidEntry {
            function,
            index,
            flags: 0,
            eax,
            ebx: 0x2222_2222,
            ecx: 0x3333_3333,
            edx: 0x4444_4444,
        }
    }

    #[test]
    fn composition_owns_and_round_trips_existing_components() {
        let guest_cpu_policy = guest_policy(vec![CpuidEntry {
            function: 0x8000_0001,
            index: 9,
            flags: 0xdead_beef,
            eax: 0x1111_1111,
            ebx: 0x2222_2222,
            ecx: 0x3333_3333,
            edx: 0x4444_4444,
        }]);
        let host_msr_model_candidate = msr_candidate(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 0x1111_2222_3333_4444),
            MsrFeatureValue::new(MSR_IA32_UCODE_REV, 0xaaaa_bbbb_cccc_dddd),
        ]);
        let expected_policy = guest_cpu_policy.clone();
        let expected_msr_candidate = host_msr_model_candidate.clone();

        let candidate = CpuModelCandidate::new(&guest_cpu_policy, &host_msr_model_candidate);
        drop(guest_cpu_policy);
        drop(host_msr_model_candidate);

        assert_eq!(candidate.guest_cpu_policy(), &expected_policy);
        assert_eq!(
            candidate.host_msr_model_candidate(),
            &expected_msr_candidate
        );
    }

    #[test]
    fn composition_retains_complete_msr_source_provenance() {
        let guest_cpu_policy = guest_policy(Vec::new());
        let host_msr_model_candidate = msr_candidate(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 1),
            MsrFeatureValue::new(MSR_IA32_UCODE_REV, 2),
        ]);
        let source_observation = host_msr_model_candidate.source_observation().clone();

        let candidate = CpuModelCandidate::new(&guest_cpu_policy, &host_msr_model_candidate);

        assert_eq!(
            candidate.host_msr_model_candidate().source_observation(),
            &source_observation
        );
        assert_eq!(
            candidate
                .host_msr_model_candidate()
                .source_observation()
                .host_mutable_values()
                .count(),
            1
        );
        assert_eq!(candidate.host_msr_model_candidate().values().len(), 1);
    }

    #[test]
    fn composition_accepts_empty_cpuid_and_empty_msr_components() {
        let guest_cpu_policy = guest_policy(Vec::new());
        let host_msr_model_candidate = msr_candidate(Vec::new());

        let candidate = CpuModelCandidate::new(&guest_cpu_policy, &host_msr_model_candidate);

        assert!(candidate.guest_cpu_policy().entries().is_empty());
        assert!(candidate.host_msr_model_candidate().values().is_empty());
        assert!(candidate
            .host_msr_model_candidate()
            .source_observation()
            .values()
            .is_empty());
    }

    #[test]
    fn cloning_composition_preserves_both_owned_contracts() {
        let guest_cpu_policy = guest_policy(vec![CpuidEntry {
            function: 7,
            index: 2,
            flags: 1,
            eax: 2,
            ebx: 3,
            ecx: 4,
            edx: 5,
        }]);
        let host_msr_model_candidate = msr_candidate(vec![MsrFeatureValue::new(
            MsrIndex::new(0x10a),
            0x1234_5678_9abc_def0,
        )]);
        let candidate = CpuModelCandidate::new(&guest_cpu_policy, &host_msr_model_candidate);

        assert_eq!(candidate.clone(), candidate);
    }

    #[test]
    fn comparison_delegates_exact_component_contracts() {
        let guest_cpu_policy = guest_policy(vec![cpuid_entry(7, 0, 1)]);
        let host_msr_model_candidate =
            msr_candidate(vec![MsrFeatureValue::new(MsrIndex::new(0x3a), 2)]);
        let reference = CpuModelCandidate::new(&guest_cpu_policy, &host_msr_model_candidate);
        let observed = reference.clone();
        let expected_cpuid = reference
            .guest_cpu_policy()
            .compare(observed.guest_cpu_policy());
        let expected_msr = reference
            .host_msr_model_candidate()
            .compare(observed.host_msr_model_candidate());

        let comparison = reference.compare(&observed);

        assert_eq!(comparison.guest_cpu_policy_comparison(), &expected_cpuid);
        assert_eq!(comparison.host_msr_model_comparison(), &expected_msr);
        assert!(comparison.guest_cpu_policy_comparison().is_exact_match());
        assert!(comparison.host_msr_model_comparison().is_exact_match());
        assert!(comparison.is_exact_match());
    }

    #[test]
    fn comparison_preserves_cpuid_only_drift() {
        let reference_policy = guest_policy(vec![cpuid_entry(7, 0, 1)]);
        let observed_policy = guest_policy(vec![cpuid_entry(7, 0, 2)]);
        let msr = msr_candidate(vec![MsrFeatureValue::new(MsrIndex::new(0x3a), 3)]);
        let reference = CpuModelCandidate::new(&reference_policy, &msr);
        let observed = CpuModelCandidate::new(&observed_policy, &msr);
        let expected_cpuid = reference
            .guest_cpu_policy()
            .compare(observed.guest_cpu_policy());
        let expected_msr = reference
            .host_msr_model_candidate()
            .compare(observed.host_msr_model_candidate());

        let comparison = reference.compare(&observed);

        assert_eq!(comparison.guest_cpu_policy_comparison(), &expected_cpuid);
        assert_eq!(comparison.host_msr_model_comparison(), &expected_msr);
        assert!(!comparison.guest_cpu_policy_comparison().is_exact_match());
        assert!(comparison.host_msr_model_comparison().is_exact_match());
        assert!(!comparison.is_exact_match());
    }

    #[test]
    fn comparison_preserves_msr_only_drift_and_source_provenance() {
        let policy = guest_policy(vec![cpuid_entry(1, 0, 1)]);
        let reference_msr = msr_candidate(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 10),
            MsrFeatureValue::new(MSR_IA32_UCODE_REV, 11),
        ]);
        let observed_msr = msr_candidate(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 20),
            MsrFeatureValue::new(MSR_IA32_UCODE_REV, 21),
        ]);
        let reference = CpuModelCandidate::new(&policy, &reference_msr);
        let observed = CpuModelCandidate::new(&policy, &observed_msr);
        let expected_cpuid = reference
            .guest_cpu_policy()
            .compare(observed.guest_cpu_policy());
        let expected_msr = reference
            .host_msr_model_candidate()
            .compare(observed.host_msr_model_candidate());

        let comparison = reference.compare(&observed);

        assert_eq!(comparison.guest_cpu_policy_comparison(), &expected_cpuid);
        assert_eq!(comparison.host_msr_model_comparison(), &expected_msr);
        assert!(comparison.guest_cpu_policy_comparison().is_exact_match());
        assert!(!comparison.host_msr_model_comparison().is_exact_match());
        assert!(!comparison.is_exact_match());
        assert_eq!(
            comparison
                .host_msr_model_comparison()
                .reference()
                .source_observation(),
            reference.host_msr_model_candidate().source_observation()
        );
        assert_eq!(
            comparison
                .host_msr_model_comparison()
                .observed()
                .source_observation(),
            observed.host_msr_model_candidate().source_observation()
        );
    }

    #[test]
    fn comparison_preserves_direction_when_both_components_drift() {
        let reference_policy = guest_policy(vec![cpuid_entry(7, 0, 1)]);
        let observed_policy = guest_policy(vec![cpuid_entry(7, 1, 1)]);
        let reference_msr = msr_candidate(vec![MsrFeatureValue::new(MsrIndex::new(0x3a), 1)]);
        let observed_msr = msr_candidate(vec![MsrFeatureValue::new(MsrIndex::new(0x10a), 2)]);
        let reference = CpuModelCandidate::new(&reference_policy, &reference_msr);
        let observed = CpuModelCandidate::new(&observed_policy, &observed_msr);

        let comparison = reference.compare(&observed);

        assert_eq!(
            comparison.guest_cpu_policy_comparison().reference(),
            reference.guest_cpu_policy()
        );
        assert_eq!(
            comparison.guest_cpu_policy_comparison().observed(),
            observed.guest_cpu_policy()
        );
        assert_eq!(
            comparison.host_msr_model_comparison().reference(),
            reference.host_msr_model_candidate()
        );
        assert_eq!(
            comparison.host_msr_model_comparison().observed(),
            observed.host_msr_model_candidate()
        );
        assert_eq!(
            comparison
                .guest_cpu_policy_comparison()
                .missing_from_observed()
                .len(),
            1
        );
        assert_eq!(
            comparison
                .guest_cpu_policy_comparison()
                .extra_in_observed()
                .len(),
            1
        );
        assert_eq!(
            comparison
                .host_msr_model_comparison()
                .missing_from_observed()
                .len(),
            1
        );
        assert_eq!(
            comparison
                .host_msr_model_comparison()
                .extra_in_observed()
                .len(),
            1
        );
        assert!(!comparison.is_exact_match());
    }

    #[test]
    fn comparison_accepts_empty_candidates_and_is_owned() {
        let policy = guest_policy(Vec::new());
        let msr = msr_candidate(Vec::new());
        let reference = CpuModelCandidate::new(&policy, &msr);
        let observed = CpuModelCandidate::new(&policy, &msr);

        let comparison = reference.compare(&observed);
        drop(reference);
        drop(observed);

        assert!(comparison.guest_cpu_policy_comparison().is_exact_match());
        assert!(comparison.host_msr_model_comparison().is_exact_match());
        assert!(comparison.is_exact_match());
        assert_eq!(comparison.clone(), comparison);
    }
}
