use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MsrIndex(u32);

impl MsrIndex {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

pub const MSR_IA32_UCODE_REV: MsrIndex = MsrIndex::new(0x0000_008b);

fn normalize_indices(indices: &[u32]) -> Vec<MsrIndex> {
    let mut seen = HashSet::with_capacity(indices.len());
    indices
        .iter()
        .copied()
        .map(MsrIndex::new)
        .filter(|index| seen.insert(*index))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostMsrIndexList {
    indices: Vec<MsrIndex>,
}

impl HostMsrIndexList {
    pub(crate) fn from_validated_raw(indices: &[u32]) -> Self {
        debug_assert!(!indices.is_empty());
        Self {
            indices: normalize_indices(indices),
        }
    }

    #[must_use]
    pub fn indices(&self) -> &[MsrIndex] {
        &self.indices
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostMsrFeatureIndexList {
    indices: Vec<MsrIndex>,
}

impl HostMsrFeatureIndexList {
    pub(crate) fn from_validated_raw(indices: &[u32]) -> Self {
        Self {
            indices: normalize_indices(indices),
        }
    }

    #[must_use]
    pub fn indices(&self) -> &[MsrIndex] {
        &self.indices
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsrFeatureStability {
    ModelImmutable,
    HostMutable,
}

const fn classify_feature_stability(index: MsrIndex) -> MsrFeatureStability {
    if index.get() == MSR_IA32_UCODE_REV.get() {
        MsrFeatureStability::HostMutable
    } else {
        MsrFeatureStability::ModelImmutable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsrFeatureValue {
    index: MsrIndex,
    value: u64,
    stability: MsrFeatureStability,
}

impl MsrFeatureValue {
    pub(crate) const fn new(index: MsrIndex, value: u64) -> Self {
        Self {
            index,
            value,
            stability: classify_feature_stability(index),
        }
    }

    #[must_use]
    pub const fn index(self) -> MsrIndex {
        self.index
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.value
    }

    #[must_use]
    pub const fn stability(self) -> MsrFeatureStability {
        self.stability
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostMsrFeatureValues {
    values: Vec<MsrFeatureValue>,
}

impl HostMsrFeatureValues {
    pub(crate) fn from_values(values: Vec<MsrFeatureValue>) -> Self {
        Self { values }
    }

    #[must_use]
    pub fn values(&self) -> &[MsrFeatureValue] {
        &self.values
    }

    pub fn model_immutable_values(&self) -> impl Iterator<Item = &MsrFeatureValue> {
        self.values
            .iter()
            .filter(|value| value.stability == MsrFeatureStability::ModelImmutable)
    }

    pub fn host_mutable_values(&self) -> impl Iterator<Item = &MsrFeatureValue> {
        self.values
            .iter()
            .filter(|value| value.stability == MsrFeatureStability::HostMutable)
    }

    #[must_use]
    pub fn model_candidate(&self) -> HostMsrModelCandidate {
        HostMsrModelCandidate::from_observation(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostMsrModelCandidate {
    source_observation: HostMsrFeatureValues,
    values: Vec<MsrFeatureValue>,
}

impl HostMsrModelCandidate {
    fn from_observation(observation: &HostMsrFeatureValues) -> Self {
        let values: Vec<MsrFeatureValue> = observation.model_immutable_values().copied().collect();
        debug_assert!(values
            .iter()
            .all(|value| value.stability() == MsrFeatureStability::ModelImmutable));

        Self {
            source_observation: observation.clone(),
            values,
        }
    }

    #[must_use]
    pub fn source_observation(&self) -> &HostMsrFeatureValues {
        &self.source_observation
    }

    #[must_use]
    pub fn values(&self) -> &[MsrFeatureValue] {
        &self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_kernel_indices_preserve_reported_order() {
        let snapshot = HostMsrIndexList::from_validated_raw(&[0x10, 0x1b, 0xc000_0080]);
        assert_eq!(
            snapshot.indices(),
            &[
                MsrIndex::new(0x10),
                MsrIndex::new(0x1b),
                MsrIndex::new(0xc000_0080),
            ]
        );
    }

    #[test]
    fn duplicate_kernel_indices_keep_first_occurrence_order() {
        let snapshot = HostMsrIndexList::from_validated_raw(&[0x10, 0x1b, 0x10, 0xc000_0080, 0x1b]);
        assert_eq!(
            snapshot.indices(),
            &[
                MsrIndex::new(0x10),
                MsrIndex::new(0x1b),
                MsrIndex::new(0xc000_0080),
            ]
        );
    }

    #[test]
    fn feature_indices_reuse_typed_values_and_preserve_order() {
        let snapshot =
            HostMsrFeatureIndexList::from_validated_raw(&[0x3a, 0x10a, 0x3a, 0x48, 0x10a]);
        assert_eq!(
            snapshot.indices(),
            &[
                MsrIndex::new(0x3a),
                MsrIndex::new(0x10a),
                MsrIndex::new(0x48),
            ]
        );
    }

    #[test]
    fn empty_feature_index_list_is_valid() {
        let snapshot = HostMsrFeatureIndexList::from_validated_raw(&[]);
        assert!(snapshot.indices().is_empty());
    }

    #[test]
    fn feature_values_preserve_index_order_data_and_stability() {
        let snapshot = HostMsrFeatureValues::from_values(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 0x1111_2222_3333_4444),
            MsrFeatureValue::new(MSR_IA32_UCODE_REV, 0xaaaa_bbbb_cccc_dddd),
        ]);
        assert_eq!(
            snapshot.values(),
            &[
                MsrFeatureValue::new(MsrIndex::new(0x3a), 0x1111_2222_3333_4444),
                MsrFeatureValue::new(MSR_IA32_UCODE_REV, 0xaaaa_bbbb_cccc_dddd),
            ]
        );
        assert_eq!(snapshot.values()[0].index(), MsrIndex::new(0x3a));
        assert_eq!(
            snapshot.values()[0].stability(),
            MsrFeatureStability::ModelImmutable
        );
        assert_eq!(snapshot.values()[1].value(), 0xaaaa_bbbb_cccc_dddd);
        assert_eq!(
            snapshot.values()[1].stability(),
            MsrFeatureStability::HostMutable
        );
    }

    #[test]
    fn ucode_revision_index_matches_x86_architecture() {
        assert_eq!(MSR_IA32_UCODE_REV.get(), 0x8b);
    }

    #[test]
    fn only_ucode_revision_is_classified_host_mutable() {
        assert_eq!(
            MsrFeatureValue::new(MSR_IA32_UCODE_REV, 1).stability(),
            MsrFeatureStability::HostMutable
        );
        assert_eq!(
            MsrFeatureValue::new(MsrIndex::new(0x3a), 1).stability(),
            MsrFeatureStability::ModelImmutable
        );
        assert_eq!(
            MsrFeatureValue::new(MsrIndex::new(0x10a), 1).stability(),
            MsrFeatureStability::ModelImmutable
        );
    }

    #[test]
    fn stability_partitions_preserve_order_and_exclude_the_other_class() {
        let immutable_a = MsrFeatureValue::new(MsrIndex::new(0x3a), 1);
        let mutable = MsrFeatureValue::new(MSR_IA32_UCODE_REV, 2);
        let immutable_b = MsrFeatureValue::new(MsrIndex::new(0x10a), 3);
        let snapshot = HostMsrFeatureValues::from_values(vec![immutable_a, mutable, immutable_b]);

        assert_eq!(
            snapshot
                .model_immutable_values()
                .copied()
                .collect::<Vec<_>>(),
            vec![immutable_a, immutable_b]
        );
        assert_eq!(
            snapshot.host_mutable_values().copied().collect::<Vec<_>>(),
            vec![mutable]
        );
    }

    #[test]
    fn empty_feature_value_snapshot_has_empty_stability_partitions() {
        let snapshot = HostMsrFeatureValues::from_values(Vec::new());
        assert!(snapshot.values().is_empty());
        assert_eq!(snapshot.model_immutable_values().count(), 0);
        assert_eq!(snapshot.host_mutable_values().count(), 0);
    }

    #[test]
    fn model_candidate_excludes_mutable_values_and_preserves_immutable_order() {
        let immutable_a = MsrFeatureValue::new(MsrIndex::new(0x3a), 0x1111);
        let mutable = MsrFeatureValue::new(MSR_IA32_UCODE_REV, 0x2222);
        let immutable_b = MsrFeatureValue::new(MsrIndex::new(0x10a), 0x3333);
        let observation =
            HostMsrFeatureValues::from_values(vec![immutable_a, mutable, immutable_b]);
        let candidate = observation.model_candidate();

        assert_eq!(candidate.values(), &[immutable_a, immutable_b]);
        assert!(candidate
            .values()
            .iter()
            .all(|value| value.stability() == MsrFeatureStability::ModelImmutable));
    }

    #[test]
    fn model_candidate_keeps_complete_owned_source_provenance() {
        let observation = HostMsrFeatureValues::from_values(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 0x1111),
            MsrFeatureValue::new(MSR_IA32_UCODE_REV, 0x2222),
        ]);
        let candidate = observation.model_candidate();

        assert_eq!(candidate.source_observation(), &observation);
        assert_eq!(
            candidate.source_observation().host_mutable_values().count(),
            1
        );
    }

    #[test]
    fn all_mutable_observation_produces_empty_candidate_with_provenance() {
        let mutable = MsrFeatureValue::new(MSR_IA32_UCODE_REV, 0x2222);
        let observation = HostMsrFeatureValues::from_values(vec![mutable]);
        let candidate = observation.model_candidate();

        assert!(candidate.values().is_empty());
        assert_eq!(candidate.source_observation().values(), &[mutable]);
    }

    #[test]
    fn empty_observation_produces_empty_candidate_and_provenance() {
        let observation = HostMsrFeatureValues::from_values(Vec::new());
        let candidate = observation.model_candidate();

        assert!(candidate.values().is_empty());
        assert!(candidate.source_observation().values().is_empty());
    }

    #[test]
    fn identical_model_candidates_compare_as_exact_match() {
        let observation = HostMsrFeatureValues::from_values(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 1),
            MsrFeatureValue::new(MsrIndex::new(0x10a), 2),
        ]);
        let reference = observation.model_candidate();
        let observed = observation.model_candidate();
        let comparison = reference.compare(&observed);

        assert!(comparison.is_exact_match());
        assert!(comparison.missing_from_observed().is_empty());
        assert!(comparison.extra_in_observed().is_empty());
        assert!(comparison.value_mismatches().is_empty());
        assert_eq!(comparison.reference(), &reference);
        assert_eq!(comparison.observed(), &observed);
    }

    #[test]
    fn model_comparison_is_order_insensitive_for_same_index_values() {
        let reference = HostMsrFeatureValues::from_values(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 1),
            MsrFeatureValue::new(MsrIndex::new(0x10a), 2),
        ])
        .model_candidate();
        let observed = HostMsrFeatureValues::from_values(vec![
            MsrFeatureValue::new(MsrIndex::new(0x10a), 2),
            MsrFeatureValue::new(MsrIndex::new(0x3a), 1),
        ])
        .model_candidate();

        assert!(reference.compare(&observed).is_exact_match());
    }

    #[test]
    fn model_comparison_reports_reference_index_missing_from_observed() {
        let missing = MsrFeatureValue::new(MsrIndex::new(0x10a), 2);
        let reference = HostMsrFeatureValues::from_values(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 1),
            missing,
        ])
        .model_candidate();
        let observed = HostMsrFeatureValues::from_values(vec![MsrFeatureValue::new(
            MsrIndex::new(0x3a),
            1,
        )])
        .model_candidate();
        let comparison = reference.compare(&observed);

        assert!(!comparison.is_exact_match());
        assert_eq!(comparison.missing_from_observed(), &[missing]);
        assert!(comparison.extra_in_observed().is_empty());
        assert!(comparison.value_mismatches().is_empty());
    }

    #[test]
    fn model_comparison_reports_observed_index_extra_to_reference() {
        let extra = MsrFeatureValue::new(MsrIndex::new(0x10a), 2);
        let reference = HostMsrFeatureValues::from_values(vec![MsrFeatureValue::new(
            MsrIndex::new(0x3a),
            1,
        )])
        .model_candidate();
        let observed = HostMsrFeatureValues::from_values(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 1),
            extra,
        ])
        .model_candidate();
        let comparison = reference.compare(&observed);

        assert!(!comparison.is_exact_match());
        assert!(comparison.missing_from_observed().is_empty());
        assert_eq!(comparison.extra_in_observed(), &[extra]);
        assert!(comparison.value_mismatches().is_empty());
    }

    #[test]
    fn model_comparison_reports_same_index_value_mismatch() {
        let index = MsrIndex::new(0x3a);
        let reference = HostMsrFeatureValues::from_values(vec![MsrFeatureValue::new(index, 1)])
            .model_candidate();
        let observed = HostMsrFeatureValues::from_values(vec![MsrFeatureValue::new(index, 2)])
            .model_candidate();
        let comparison = reference.compare(&observed);

        assert_eq!(
            comparison.value_mismatches(),
            &[MsrModelValueMismatch::new(index, 1, 2)]
        );
        assert!(comparison.missing_from_observed().is_empty());
        assert!(comparison.extra_in_observed().is_empty());
    }

    #[test]
    fn model_comparison_ignores_host_mutable_source_drift_but_keeps_provenance() {
        let immutable = MsrFeatureValue::new(MsrIndex::new(0x3a), 7);
        let reference_observation = HostMsrFeatureValues::from_values(vec![
            immutable,
            MsrFeatureValue::new(MSR_IA32_UCODE_REV, 0x1111),
        ]);
        let observed_observation = HostMsrFeatureValues::from_values(vec![
            MsrFeatureValue::new(MSR_IA32_UCODE_REV, 0x2222),
            immutable,
        ]);
        let reference = reference_observation.model_candidate();
        let observed = observed_observation.model_candidate();
        let comparison = reference.compare(&observed);

        assert!(comparison.is_exact_match());
        assert_eq!(
            comparison.reference().source_observation(),
            &reference_observation
        );
        assert_eq!(
            comparison.observed().source_observation(),
            &observed_observation
        );
        assert_ne!(
            comparison.reference().source_observation(),
            comparison.observed().source_observation()
        );
    }

    #[test]
    fn model_comparison_reports_mixed_differences_in_source_order() {
        let shared = MsrFeatureValue::new(MsrIndex::new(0x3a), 1);
        let missing = MsrFeatureValue::new(MsrIndex::new(0x48), 2);
        let mismatch_reference = MsrFeatureValue::new(MsrIndex::new(0x10a), 3);
        let mismatch_observed = MsrFeatureValue::new(MsrIndex::new(0x10a), 4);
        let extra = MsrFeatureValue::new(MsrIndex::new(0x122), 5);
        let reference =
            HostMsrFeatureValues::from_values(vec![shared, missing, mismatch_reference])
                .model_candidate();
        let observed =
            HostMsrFeatureValues::from_values(vec![mismatch_observed, shared, extra])
                .model_candidate();
        let comparison = reference.compare(&observed);

        assert_eq!(comparison.missing_from_observed(), &[missing]);
        assert_eq!(comparison.extra_in_observed(), &[extra]);
        assert_eq!(
            comparison.value_mismatches(),
            &[MsrModelValueMismatch::new(MsrIndex::new(0x10a), 3, 4)]
        );
    }

    #[test]
    fn empty_model_candidates_compare_as_exact_match() {
        let reference = HostMsrFeatureValues::from_values(Vec::new()).model_candidate();
        let observed = HostMsrFeatureValues::from_values(Vec::new()).model_candidate();

        assert!(reference.compare(&observed).is_exact_match());
    }

    #[test]
    fn msr_index_round_trips_raw_value() {
        let index = MsrIndex::new(0xdead_beef);
        assert_eq!(index.get(), 0xdead_beef);
    }
}
