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
