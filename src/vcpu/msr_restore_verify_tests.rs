use super::*;
use crate::error::{Error, HostEnvironmentError};
use crate::kvm::msr::{GuestMsrAccessPolicy, GuestMsrValueSet, HostMsrIndexList, MsrIndex};
use std::cell::RefCell;
use std::io;

fn policy(indices: &[u32]) -> GuestMsrAccessPolicy {
    let host = HostMsrIndexList::from_validated_raw(indices);
    let requested: Vec<MsrIndex> = indices.iter().copied().map(MsrIndex::new).collect();
    GuestMsrAccessPolicy::from_host(&host, &requested).unwrap()
}

fn snapshot(indices: &[u32], data: &[u64]) -> GuestMsrSnapshot {
    assert_eq!(indices.len(), data.len());
    let policy = policy(indices);
    let requested: Vec<(MsrIndex, u64)> = indices
        .iter()
        .copied()
        .map(MsrIndex::new)
        .zip(data.iter().copied())
        .collect();
    let values = GuestMsrValueSet::from_policy(&policy, &requested).unwrap();
    GuestMsrSnapshot::from_capture(&policy, &values).unwrap()
}

fn empty_snapshot() -> GuestMsrSnapshot {
    let host = HostMsrIndexList::from_validated_raw(&[0x10]);
    let policy = GuestMsrAccessPolicy::from_host(&host, &[]).unwrap();
    let values = GuestMsrValueSet::from_policy(&policy, &[]).unwrap();
    GuestMsrSnapshot::from_capture(&policy, &values).unwrap()
}

#[test]
fn restore_verification_writes_before_recapturing() {
    let expected = snapshot(&[0x10, 0x1b], &[11, 22]);
    let observed = expected.clone();
    let sequence = RefCell::new(Vec::new());

    let comparison = restore_and_verify_msr_snapshot_with(
        &expected,
        |snapshot| {
            sequence.borrow_mut().push("write");
            assert_eq!(snapshot, &expected);
            Ok(())
        },
        |policy| {
            sequence.borrow_mut().push("read");
            assert_eq!(policy, expected.policy());
            Ok(observed.clone())
        },
    )
    .unwrap();

    assert_eq!(&*sequence.borrow(), &["write", "read"]);
    assert!(comparison.is_exact_match());
}

#[test]
fn empty_snapshot_restore_verification_still_delegates_once_each() {
    let expected = empty_snapshot();
    let mut writes = 0;
    let mut reads = 0;

    let comparison = restore_and_verify_msr_snapshot_with(
        &expected,
        |snapshot| {
            writes += 1;
            assert!(snapshot.values().values().is_empty());
            Ok(())
        },
        |policy| {
            reads += 1;
            assert!(policy.entries().is_empty());
            Ok(expected.clone())
        },
    )
    .unwrap();

    assert_eq!(writes, 1);
    assert_eq!(reads, 1);
    assert!(comparison.is_exact_match());
}

#[test]
fn write_failure_propagates_unchanged_and_skips_recapture() {
    let expected = snapshot(&[0x10, 0x1b], &[11, 22]);
    let mut reads = 0;

    let error = restore_and_verify_msr_snapshot_with(
        &expected,
        |_snapshot| {
            Err(Error::HostEnvironment(
                HostEnvironmentError::VcpuMsrPartialWrite {
                    id: 7,
                    requested: 2,
                    processed: 1,
                    first_unwritten_index: 0x1b,
                },
            ))
        },
        |_policy| {
            reads += 1;
            Ok(expected.clone())
        },
    )
    .unwrap_err();

    assert_eq!(reads, 0);
    assert!(matches!(
        error,
        Error::HostEnvironment(HostEnvironmentError::VcpuMsrPartialWrite {
            id: 7,
            requested: 2,
            processed: 1,
            first_unwritten_index: 0x1b,
        })
    ));
}

#[test]
fn recapture_failure_after_successful_write_propagates_without_retry() {
    let expected = snapshot(&[0x10], &[11]);
    let mut writes = 0;
    let mut reads = 0;

    let error = restore_and_verify_msr_snapshot_with(
        &expected,
        |_snapshot| {
            writes += 1;
            Ok(())
        },
        |_policy| {
            reads += 1;
            Err(Error::HostEnvironment(
                HostEnvironmentError::VcpuOperation {
                    id: 9,
                    operation: "capture-test",
                    source: io::Error::other("capture failed"),
                },
            ))
        },
    )
    .unwrap_err();

    assert_eq!(writes, 1);
    assert_eq!(reads, 1);
    assert!(matches!(
        error,
        Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
            id: 9,
            operation: "capture-test",
            ..
        })
    ));
}

#[test]
fn exact_recapture_returns_owned_exact_comparison() {
    let expected = snapshot(&[0xc000_0080, 0x10], &[0xaaaa, 0x1111]);
    let observed = expected.clone();

    let comparison = restore_and_verify_msr_snapshot_with(
        &expected,
        |_snapshot| Ok(()),
        |_policy| Ok(observed.clone()),
    )
    .unwrap();

    assert!(comparison.is_exact_match());
    assert_eq!(comparison.reference(), &expected);
    assert_eq!(comparison.observed(), &observed);
}

#[test]
fn recapture_mismatch_is_reported_without_repair_or_retry() {
    let expected = snapshot(&[0x10, 0x1b], &[11, 22]);
    let observed = snapshot(&[0x10, 0x1b], &[11, 99]);
    let mut writes = 0;
    let mut reads = 0;

    let comparison = restore_and_verify_msr_snapshot_with(
        &expected,
        |_snapshot| {
            writes += 1;
            Ok(())
        },
        |_policy| {
            reads += 1;
            Ok(observed.clone())
        },
    )
    .unwrap();

    assert_eq!(writes, 1);
    assert_eq!(reads, 1);
    assert!(!comparison.is_exact_match());
    assert!(comparison.policy_matches());
    assert_eq!(comparison.value_mismatches().len(), 1);
    let mismatch = comparison.value_mismatches()[0];
    assert_eq!(mismatch.position(), 1);
    assert_eq!(mismatch.index(), MsrIndex::new(0x1b));
    assert_eq!(mismatch.reference_value(), 22);
    assert_eq!(mismatch.observed_value(), 99);
}
