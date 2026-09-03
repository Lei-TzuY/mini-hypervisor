use crate::kvm::cpu::GuestCpuPolicy;
use crate::kvm::msr::HostMsrModelCandidate;

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
}
