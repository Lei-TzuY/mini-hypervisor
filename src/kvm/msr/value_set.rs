use super::{GuestMsrAccessPolicy, MsrAccessAuthority, MsrIndex};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestMsrValue {
    index: MsrIndex,
    value: u64,
}

impl GuestMsrValue {
    pub(super) const fn new(index: MsrIndex, value: u64) -> Self {
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
pub struct GuestMsrValueSet {
    values: Vec<GuestMsrValue>,
}

impl GuestMsrValueSet {
    pub fn from_policy(
        policy: &GuestMsrAccessPolicy,
        requested: &[(MsrIndex, u64)],
    ) -> Result<Self, GuestMsrValueSetError> {
        let mut seen = HashMap::with_capacity(requested.len());
        let mut values = Vec::with_capacity(requested.len());

        for (position, (index, value)) in requested.iter().copied().enumerate() {
            if let Some(first_position) = seen.get(&index).copied() {
                return Err(GuestMsrValueSetError::DuplicateIndex {
                    index,
                    first_position,
                    duplicate_position: position,
                });
            }

            let authorized = policy.entries().iter().any(|entry| {
                entry.index() == index && entry.authority() == MsrAccessAuthority::ReadWrite
            });
            if !authorized {
                return Err(GuestMsrValueSetError::UnauthorizedIndex { index, position });
            }

            seen.insert(index, position);
            values.push(GuestMsrValue::new(index, value));
        }

        Ok(Self { values })
    }

    #[must_use]
    pub fn values(&self) -> &[GuestMsrValue] {
        &self.values
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestMsrValueSetError {
    UnauthorizedIndex {
        index: MsrIndex,
        position: usize,
    },
    DuplicateIndex {
        index: MsrIndex,
        first_position: usize,
        duplicate_position: usize,
    },
}

impl std::fmt::Display for GuestMsrValueSetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnauthorizedIndex { index, position } => write!(
                f,
                "guest MSR value index {:#x} at position {position} is not authorized by the guest MSR access policy",
                index.get()
            ),
            Self::DuplicateIndex {
                index,
                first_position,
                duplicate_position,
            } => write!(
                f,
                "guest MSR value index {:#x} is duplicated at positions {first_position} and {duplicate_position}",
                index.get()
            ),
        }
    }
}

impl std::error::Error for GuestMsrValueSetError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestMsrSnapshot {
    policy: GuestMsrAccessPolicy,
    values: GuestMsrValueSet,
}

impl GuestMsrSnapshot {
    pub(crate) fn from_capture(
        policy: &GuestMsrAccessPolicy,
        values: &GuestMsrValueSet,
    ) -> Result<Self, GuestMsrSnapshotError> {
        if policy.entries().len() != values.values().len() {
            return Err(GuestMsrSnapshotError::CoverageMismatch {
                policy_entries: policy.entries().len(),
                value_entries: values.values().len(),
            });
        }

        for (position, (policy_entry, value)) in policy
            .entries()
            .iter()
            .zip(values.values().iter())
            .enumerate()
        {
            if policy_entry.index() != value.index() {
                return Err(GuestMsrSnapshotError::IndexMismatch {
                    position,
                    policy_index: policy_entry.index(),
                    value_index: value.index(),
                });
            }
        }

        Ok(Self {
            policy: policy.clone(),
            values: values.clone(),
        })
    }

    #[must_use]
    pub fn policy(&self) -> &GuestMsrAccessPolicy {
        &self.policy
    }

    #[must_use]
    pub fn values(&self) -> &GuestMsrValueSet {
        &self.values
    }

    #[must_use]
    pub fn compare(&self, observed: &Self) -> GuestMsrSnapshotComparison {
        let policy_matches = self.policy == observed.policy;
        let value_mismatches = if policy_matches {
            self.values
                .values()
                .iter()
                .zip(observed.values.values().iter())
                .enumerate()
                .filter_map(|(position, (reference, candidate))| {
                    (reference.value() != candidate.value()).then_some(
                        GuestMsrSnapshotValueMismatch::new(
                            position,
                            reference.index(),
                            reference.value(),
                            candidate.value(),
                        ),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };

        GuestMsrSnapshotComparison {
            reference: self.clone(),
            observed: observed.clone(),
            policy_matches,
            value_mismatches,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestMsrSnapshotValueMismatch {
    position: usize,
    index: MsrIndex,
    reference_value: u64,
    observed_value: u64,
}

impl GuestMsrSnapshotValueMismatch {
    const fn new(
        position: usize,
        index: MsrIndex,
        reference_value: u64,
        observed_value: u64,
    ) -> Self {
        Self {
            position,
            index,
            reference_value,
            observed_value,
        }
    }

    #[must_use]
    pub const fn position(self) -> usize {
        self.position
    }

    #[must_use]
    pub const fn index(self) -> MsrIndex {
        self.index
    }

    #[must_use]
    pub const fn reference_value(self) -> u64 {
        self.reference_value
    }

    #[must_use]
    pub const fn observed_value(self) -> u64 {
        self.observed_value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestMsrSnapshotComparison {
    reference: GuestMsrSnapshot,
    observed: GuestMsrSnapshot,
    policy_matches: bool,
    value_mismatches: Vec<GuestMsrSnapshotValueMismatch>,
}

impl GuestMsrSnapshotComparison {
    #[must_use]
    pub fn reference(&self) -> &GuestMsrSnapshot {
        &self.reference
    }

    #[must_use]
    pub fn observed(&self) -> &GuestMsrSnapshot {
        &self.observed
    }

    #[must_use]
    pub const fn policy_matches(&self) -> bool {
        self.policy_matches
    }

    #[must_use]
    pub fn value_mismatches(&self) -> &[GuestMsrSnapshotValueMismatch] {
        &self.value_mismatches
    }

    #[must_use]
    pub fn is_exact_match(&self) -> bool {
        self.policy_matches && self.value_mismatches.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestMsrSnapshotError {
    CoverageMismatch {
        policy_entries: usize,
        value_entries: usize,
    },
    IndexMismatch {
        position: usize,
        policy_index: MsrIndex,
        value_index: MsrIndex,
    },
}

impl std::fmt::Display for GuestMsrSnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CoverageMismatch {
                policy_entries,
                value_entries,
            } => write!(
                f,
                "full guest MSR snapshot has {value_entries} values for {policy_entries} policy entries"
            ),
            Self::IndexMismatch {
                position,
                policy_index,
                value_index,
            } => write!(
                f,
                "full guest MSR snapshot index mismatch at position {position}: policy {:#x}, values {:#x}",
                policy_index.get(),
                value_index.get()
            ),
        }
    }
}

impl std::error::Error for GuestMsrSnapshotError {}

#[cfg(test)]
mod snapshot_comparison_tests {
    use super::*;
    use crate::kvm::msr::HostMsrIndexList;

    fn snapshot(indices: &[u32], data: &[u64]) -> GuestMsrSnapshot {
        assert_eq!(indices.len(), data.len());
        let host = HostMsrIndexList::from_validated_raw(indices);
        let requested: Vec<MsrIndex> = indices.iter().copied().map(MsrIndex::new).collect();
        let policy = GuestMsrAccessPolicy::from_host(&host, &requested).unwrap();
        let pairs: Vec<(MsrIndex, u64)> = requested
            .iter()
            .copied()
            .zip(data.iter().copied())
            .collect();
        let values = GuestMsrValueSet::from_policy(&policy, &pairs).unwrap();
        GuestMsrSnapshot::from_capture(&policy, &values).unwrap()
    }

    fn empty_snapshot() -> GuestMsrSnapshot {
        let host = HostMsrIndexList::from_validated_raw(&[0x10]);
        let policy = GuestMsrAccessPolicy::from_host(&host, &[]).unwrap();
        let values = GuestMsrValueSet::from_policy(&policy, &[]).unwrap();
        GuestMsrSnapshot::from_capture(&policy, &values).unwrap()
    }

    #[test]
    fn identical_snapshots_compare_as_exact_match() {
        let reference = snapshot(&[0x10, 0x1b], &[11, 22]);
        let observed = reference.clone();

        let comparison = reference.compare(&observed);

        assert!(comparison.policy_matches());
        assert!(comparison.value_mismatches().is_empty());
        assert!(comparison.is_exact_match());
        assert_eq!(comparison.reference(), &reference);
        assert_eq!(comparison.observed(), &observed);
    }

    #[test]
    fn empty_snapshots_compare_as_exact_match() {
        let reference = empty_snapshot();
        let observed = empty_snapshot();

        let comparison = reference.compare(&observed);

        assert!(comparison.policy_matches());
        assert!(comparison.value_mismatches().is_empty());
        assert!(comparison.is_exact_match());
    }

    #[test]
    fn policy_mismatch_stops_value_level_comparison() {
        let reference = snapshot(&[0x10, 0x1b], &[11, 22]);
        let observed = snapshot(&[0x10], &[99]);

        let comparison = reference.compare(&observed);

        assert!(!comparison.policy_matches());
        assert!(comparison.value_mismatches().is_empty());
        assert!(!comparison.is_exact_match());
        assert_eq!(comparison.reference(), &reference);
        assert_eq!(comparison.observed(), &observed);
    }

    #[test]
    fn same_policy_reports_positional_value_mismatch() {
        let reference = snapshot(&[0xc000_0080, 0x10], &[0xaaaa, 0x1111]);
        let observed = snapshot(&[0xc000_0080, 0x10], &[0xbbbb, 0x1111]);

        let comparison = reference.compare(&observed);

        assert!(comparison.policy_matches());
        assert_eq!(
            comparison.value_mismatches(),
            &[GuestMsrSnapshotValueMismatch::new(
                0,
                MsrIndex::new(0xc000_0080),
                0xaaaa,
                0xbbbb,
            )]
        );
        assert!(!comparison.is_exact_match());
    }

    #[test]
    fn comparison_owns_both_snapshots_after_sources_drop() {
        let comparison = {
            let reference = snapshot(&[0x10], &[1]);
            let observed = snapshot(&[0x10], &[2]);
            reference.compare(&observed)
        };

        assert_eq!(comparison.reference().values().values()[0].value(), 1);
        assert_eq!(comparison.observed().values().values()[0].value(), 2);
        assert_eq!(comparison.value_mismatches().len(), 1);
    }
}
