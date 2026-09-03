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
pub struct MsrFeatureValue {
    index: MsrIndex,
    value: u64,
}

impl MsrFeatureValue {
    pub(crate) const fn new(index: MsrIndex, value: u64) -> Self {
        Self { index, value }
    }

    #[must_use]
    pub const fn index(self) -> MsrIndex {
        self.index
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.value
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
    fn feature_values_preserve_index_order_and_data() {
        let snapshot = HostMsrFeatureValues::from_values(vec![
            MsrFeatureValue::new(MsrIndex::new(0x3a), 0x1111_2222_3333_4444),
            MsrFeatureValue::new(MsrIndex::new(0x10a), 0xaaaa_bbbb_cccc_dddd),
        ]);
        assert_eq!(
            snapshot.values(),
            &[
                MsrFeatureValue::new(MsrIndex::new(0x3a), 0x1111_2222_3333_4444),
                MsrFeatureValue::new(MsrIndex::new(0x10a), 0xaaaa_bbbb_cccc_dddd),
            ]
        );
        assert_eq!(snapshot.values()[0].index(), MsrIndex::new(0x3a));
        assert_eq!(snapshot.values()[1].value(), 0xaaaa_bbbb_cccc_dddd);
    }

    #[test]
    fn empty_feature_value_snapshot_is_valid() {
        let snapshot = HostMsrFeatureValues::from_values(Vec::new());
        assert!(snapshot.values().is_empty());
    }

    #[test]
    fn msr_index_round_trips_raw_value() {
        let index = MsrIndex::new(0xdead_beef);
        assert_eq!(index.get(), 0xdead_beef);
    }
}
