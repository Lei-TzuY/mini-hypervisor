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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostMsrIndexList {
    indices: Vec<MsrIndex>,
}

impl HostMsrIndexList {
    pub(crate) fn from_validated_raw(indices: &[u32]) -> Self {
        debug_assert!(!indices.is_empty());
        let mut seen = HashSet::with_capacity(indices.len());
        let indices = indices
            .iter()
            .copied()
            .map(MsrIndex::new)
            .filter(|index| seen.insert(*index))
            .collect();
        Self { indices }
    }

    #[must_use]
    pub fn indices(&self) -> &[MsrIndex] {
        &self.indices
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
        let snapshot =
            HostMsrIndexList::from_validated_raw(&[0x10, 0x1b, 0x10, 0xc000_0080, 0x1b]);
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
    fn msr_index_round_trips_raw_value() {
        let index = MsrIndex::new(0xdead_beef);
        assert_eq!(index.get(), 0xdead_beef);
    }
}
