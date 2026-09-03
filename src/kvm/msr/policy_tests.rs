use super::*;

fn host(indices: &[u32]) -> HostMsrIndexList {
    HostMsrIndexList::from_validated_raw(indices)
}

#[test]
fn empty_guest_policy_is_valid_and_owned() {
    let host = host(&[0x10, 0x1b]);
    let policy = GuestMsrAccessPolicy::from_host(&host, &[]).unwrap();

    assert!(policy.entries().is_empty());
}

#[test]
fn guest_policy_preserves_caller_order_with_explicit_read_write_authority() {
    let host = host(&[0x10, 0x1b, 0xc000_0080]);
    let requested = [MsrIndex::new(0xc000_0080), MsrIndex::new(0x10)];

    let policy = GuestMsrAccessPolicy::from_host(&host, &requested).unwrap();

    assert_eq!(
        policy.entries(),
        &[
            GuestMsrAccess::new(MsrIndex::new(0xc000_0080), MsrAccessAuthority::ReadWrite),
            GuestMsrAccess::new(MsrIndex::new(0x10), MsrAccessAuthority::ReadWrite),
        ]
    );
}

#[test]
fn guest_policy_rejects_unsupported_index_without_partial_policy() {
    let host = host(&[0x10, 0x1b]);
    let requested = [
        MsrIndex::new(0x10),
        MsrIndex::new(0xdead_beef),
        MsrIndex::new(0x1b),
    ];

    assert_eq!(
        GuestMsrAccessPolicy::from_host(&host, &requested),
        Err(GuestMsrPolicyError::UnsupportedIndex {
            index: MsrIndex::new(0xdead_beef),
            position: 1,
        })
    );
}

#[test]
fn guest_policy_rejects_duplicate_index_and_reports_both_positions() {
    let host = host(&[0x10, 0x1b]);
    let requested = [
        MsrIndex::new(0x1b),
        MsrIndex::new(0x10),
        MsrIndex::new(0x1b),
    ];

    assert_eq!(
        GuestMsrAccessPolicy::from_host(&host, &requested),
        Err(GuestMsrPolicyError::DuplicateIndex {
            index: MsrIndex::new(0x1b),
            first_position: 0,
            duplicate_position: 2,
        })
    );
}

#[test]
fn guest_policy_owns_entries_after_sources_are_dropped() {
    let policy = {
        let host = host(&[0x10, 0x1b]);
        let requested = vec![MsrIndex::new(0x1b), MsrIndex::new(0x10)];
        GuestMsrAccessPolicy::from_host(&host, &requested).unwrap()
    };

    assert_eq!(policy.entries()[0].index(), MsrIndex::new(0x1b));
    assert_eq!(policy.entries()[0].authority(), MsrAccessAuthority::ReadWrite);
    assert_eq!(policy.entries()[1].index(), MsrIndex::new(0x10));
}
