use super::value_set::{GuestMsrSnapshot, GuestMsrSnapshotError};
use super::*;

fn policy(indices: &[u32]) -> GuestMsrAccessPolicy {
    let host = HostMsrIndexList::from_validated_raw(indices);
    let requested: Vec<MsrIndex> = indices.iter().copied().map(MsrIndex::new).collect();
    GuestMsrAccessPolicy::from_host(&host, &requested).unwrap()
}

#[test]
fn empty_guest_value_subset_is_valid() {
    let policy = policy(&[0x10, 0x1b]);
    let values = GuestMsrValueSet::from_policy(&policy, &[]).unwrap();

    assert!(values.values().is_empty());
}

#[test]
fn guest_value_set_accepts_authorized_subset_and_preserves_caller_order() {
    let policy = policy(&[0x10, 0x1b, 0xc000_0080]);
    let requested = [
        (MsrIndex::new(0xc000_0080), 0xaaaa_bbbb_cccc_dddd),
        (MsrIndex::new(0x10), 0x1111_2222_3333_4444),
    ];

    let values = GuestMsrValueSet::from_policy(&policy, &requested).unwrap();

    assert_eq!(
        values.values(),
        &[
            GuestMsrValue::new(MsrIndex::new(0xc000_0080), 0xaaaa_bbbb_cccc_dddd),
            GuestMsrValue::new(MsrIndex::new(0x10), 0x1111_2222_3333_4444),
        ]
    );
}

#[test]
fn guest_value_set_rejects_unauthorized_index_without_partial_state() {
    let policy = policy(&[0x10]);
    let requested = [(MsrIndex::new(0x10), 1), (MsrIndex::new(0x1b), 2)];

    assert_eq!(
        GuestMsrValueSet::from_policy(&policy, &requested),
        Err(GuestMsrValueSetError::UnauthorizedIndex {
            index: MsrIndex::new(0x1b),
            position: 1,
        })
    );
}

#[test]
fn guest_value_set_rejects_duplicate_index_and_reports_both_positions() {
    let policy = policy(&[0x10, 0x1b]);
    let requested = [
        (MsrIndex::new(0x1b), 1),
        (MsrIndex::new(0x10), 2),
        (MsrIndex::new(0x1b), 3),
    ];

    assert_eq!(
        GuestMsrValueSet::from_policy(&policy, &requested),
        Err(GuestMsrValueSetError::DuplicateIndex {
            index: MsrIndex::new(0x1b),
            first_position: 0,
            duplicate_position: 2,
        })
    );
}

#[test]
fn guest_value_set_owns_values_after_sources_are_dropped() {
    let values = {
        let policy = policy(&[0x10, 0x1b]);
        let requested = vec![
            (MsrIndex::new(0x1b), 0xaaaa_bbbb_cccc_dddd),
            (MsrIndex::new(0x10), 0x1111_2222_3333_4444),
        ];
        GuestMsrValueSet::from_policy(&policy, &requested).unwrap()
    };

    assert_eq!(values.values()[0].index(), MsrIndex::new(0x1b));
    assert_eq!(values.values()[0].value(), 0xaaaa_bbbb_cccc_dddd);
    assert_eq!(values.values()[1].index(), MsrIndex::new(0x10));
    assert_eq!(values.values()[1].value(), 0x1111_2222_3333_4444);
}

#[test]
fn empty_full_policy_snapshot_is_valid() {
    let policy = policy(&[]);
    let values = GuestMsrValueSet::from_policy(&policy, &[]).unwrap();

    let snapshot = GuestMsrSnapshot::from_capture(&policy, &values).unwrap();

    assert!(snapshot.policy().entries().is_empty());
    assert!(snapshot.values().values().is_empty());
}

#[test]
fn full_policy_snapshot_preserves_policy_value_order_and_ownership() {
    let snapshot = {
        let policy = policy(&[0x10a, 0x3a]);
        let requested = [
            (MsrIndex::new(0x10a), 0x1111_2222_3333_4444),
            (MsrIndex::new(0x3a), 0xaaaa_bbbb_cccc_dddd),
        ];
        let values = GuestMsrValueSet::from_policy(&policy, &requested).unwrap();
        GuestMsrSnapshot::from_capture(&policy, &values).unwrap()
    };

    assert_eq!(snapshot.policy().entries()[0].index(), MsrIndex::new(0x10a));
    assert_eq!(snapshot.policy().entries()[1].index(), MsrIndex::new(0x3a));
    assert_eq!(snapshot.values().values()[0].index(), MsrIndex::new(0x10a));
    assert_eq!(snapshot.values().values()[0].value(), 0x1111_2222_3333_4444);
    assert_eq!(snapshot.values().values()[1].index(), MsrIndex::new(0x3a));
    assert_eq!(snapshot.values().values()[1].value(), 0xaaaa_bbbb_cccc_dddd);
}

#[test]
fn full_policy_snapshot_rejects_missing_value() {
    let policy = policy(&[0x10, 0x1b]);
    let values = GuestMsrValueSet::from_policy(&policy, &[(MsrIndex::new(0x10), 1)]).unwrap();

    assert_eq!(
        GuestMsrSnapshot::from_capture(&policy, &values),
        Err(GuestMsrSnapshotError::CoverageMismatch {
            policy_entries: 2,
            value_entries: 1,
        })
    );
}

#[test]
fn full_policy_snapshot_rejects_extra_value_from_another_policy() {
    let snapshot_policy = policy(&[0x10]);
    let broader_policy = policy(&[0x10, 0x1b]);
    let values = GuestMsrValueSet::from_policy(
        &broader_policy,
        &[(MsrIndex::new(0x10), 1), (MsrIndex::new(0x1b), 2)],
    )
    .unwrap();

    assert_eq!(
        GuestMsrSnapshot::from_capture(&snapshot_policy, &values),
        Err(GuestMsrSnapshotError::CoverageMismatch {
            policy_entries: 1,
            value_entries: 2,
        })
    );
}

#[test]
fn full_policy_snapshot_rejects_reordered_values() {
    let policy = policy(&[0x10, 0x1b]);
    let values = GuestMsrValueSet::from_policy(
        &policy,
        &[(MsrIndex::new(0x1b), 2), (MsrIndex::new(0x10), 1)],
    )
    .unwrap();

    assert_eq!(
        GuestMsrSnapshot::from_capture(&policy, &values),
        Err(GuestMsrSnapshotError::IndexMismatch {
            position: 0,
            policy_index: MsrIndex::new(0x10),
            value_index: MsrIndex::new(0x1b),
        })
    );
}
