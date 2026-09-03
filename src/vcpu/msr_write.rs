use crate::error::{Error, HostEnvironmentError};
use crate::kvm::msr::value_set::GuestMsrSnapshot;
use crate::kvm::msr::GuestMsrValueSet;
use crate::kvm::sys;
use crate::vcpu::{vcpu_operation, Vcpu, VcpuId};
use std::io;
use std::os::fd::AsRawFd;

const VCPU_MSR_WRITE_CAPACITY: usize = 1024;

impl Vcpu {
    pub fn set_msrs(&self, values: &GuestMsrValueSet) -> Result<(), Error> {
        let Some(request) = prepare_vcpu_msr_write_request(values)
            .map_err(|source| vcpu_operation(self.id, "validate KVM_SET_MSRS request", source))?
        else {
            return Ok(());
        };

        let processed = sys::set_msrs(self.fd.as_raw_fd(), &request)
            .map_err(|source| vcpu_operation(self.id, "KVM_SET_MSRS", source))?;
        validate_vcpu_msr_write_completion(self.id, values, processed)
    }

    pub fn restore_msr_snapshot(&self, snapshot: &GuestMsrSnapshot) -> Result<(), Error> {
        restore_msr_snapshot_with(snapshot, |values| self.set_msrs(values))
    }
}

fn restore_msr_snapshot_with<F>(snapshot: &GuestMsrSnapshot, mut write: F) -> Result<(), Error>
where
    F: FnMut(&GuestMsrValueSet) -> Result<(), Error>,
{
    write(snapshot.values())
}

fn prepare_vcpu_msr_write_request(
    values: &GuestMsrValueSet,
) -> io::Result<Option<sys::KvmMsrs<VCPU_MSR_WRITE_CAPACITY>>> {
    let values = values.values();
    if values.is_empty() {
        return Ok(None);
    }
    if values.len() > VCPU_MSR_WRITE_CAPACITY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "vCPU KVM_SET_MSRS request count {} exceeds bounded capacity {}",
                values.len(),
                VCPU_MSR_WRITE_CAPACITY
            ),
        ));
    }

    let mut request = sys::KvmMsrs::<VCPU_MSR_WRITE_CAPACITY>::new();
    request.nmsrs =
        u32::try_from(values.len()).expect("bounded vCPU MSR write count always fits u32");
    for (entry, value) in request.entries[..values.len()]
        .iter_mut()
        .zip(values.iter().copied())
    {
        entry.index = value.index().get();
        entry.data = value.value();
    }
    Ok(Some(request))
}

fn validate_vcpu_msr_write_completion(
    id: VcpuId,
    values: &GuestMsrValueSet,
    processed: usize,
) -> Result<(), Error> {
    let requested = values.values().len();
    if processed == requested {
        return Ok(());
    }
    if processed < requested {
        return Err(Error::HostEnvironment(
            HostEnvironmentError::VcpuMsrPartialWrite {
                id: id.get(),
                requested,
                processed,
                first_unwritten_index: values.values()[processed].index().get(),
            },
        ));
    }

    Err(Error::HostEnvironment(
        HostEnvironmentError::VcpuMsrInvalidWriteCompletion {
            id: id.get(),
            requested,
            processed,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{Error, HostEnvironmentError};
    use crate::kvm::msr::{GuestMsrAccessPolicy, GuestMsrValueSet, HostMsrIndexList, MsrIndex};
    use crate::kvm::sys;
    use crate::vcpu::VcpuId;
    use std::io;

    fn policy(indices: &[u32]) -> GuestMsrAccessPolicy {
        let host = HostMsrIndexList::from_validated_raw(indices);
        let requested: Vec<MsrIndex> = indices.iter().copied().map(MsrIndex::new).collect();
        GuestMsrAccessPolicy::from_host(&host, &requested).unwrap()
    }

    fn value_set(policy_indices: &[u32], values: &[(u32, u64)]) -> GuestMsrValueSet {
        let policy = policy(policy_indices);
        let requested: Vec<(MsrIndex, u64)> = values
            .iter()
            .copied()
            .map(|(index, value)| (MsrIndex::new(index), value))
            .collect();
        GuestMsrValueSet::from_policy(&policy, &requested).unwrap()
    }

    fn snapshot(indices: &[u32], values: &[(u32, u64)]) -> GuestMsrSnapshot {
        let policy = policy(indices);
        let requested: Vec<(MsrIndex, u64)> = values
            .iter()
            .copied()
            .map(|(index, value)| (MsrIndex::new(index), value))
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
    fn snapshot_restore_delegates_exactly_once_with_snapshot_value_order() {
        let snapshot = snapshot(
            &[0xc000_0080, 0x10],
            &[
                (0xc000_0080, 0xaaaa_bbbb_cccc_dddd),
                (0x10, 0x1111_2222_3333_4444),
            ],
        );
        let mut calls = 0;

        restore_msr_snapshot_with(&snapshot, |values| {
            calls += 1;
            assert_eq!(values, snapshot.values());
            assert_eq!(values.values()[0].index(), MsrIndex::new(0xc000_0080));
            assert_eq!(values.values()[0].value(), 0xaaaa_bbbb_cccc_dddd);
            assert_eq!(values.values()[1].index(), MsrIndex::new(0x10));
            assert_eq!(values.values()[1].value(), 0x1111_2222_3333_4444);
            Ok(())
        })
        .unwrap();

        assert_eq!(calls, 1);
    }

    #[test]
    fn empty_snapshot_restore_still_delegates_once_to_existing_empty_write_path() {
        let snapshot = empty_snapshot();
        let mut calls = 0;

        restore_msr_snapshot_with(&snapshot, |values| {
            calls += 1;
            assert!(values.values().is_empty());
            Ok(())
        })
        .unwrap();

        assert_eq!(calls, 1);
    }

    #[test]
    fn snapshot_restore_propagates_partial_write_diagnostics_unchanged() {
        let snapshot = snapshot(&[0x10, 0x1b], &[(0x10, 1), (0x1b, 2)]);

        let error = restore_msr_snapshot_with(&snapshot, |_values| {
            Err(Error::HostEnvironment(
                HostEnvironmentError::VcpuMsrPartialWrite {
                    id: 7,
                    requested: 2,
                    processed: 1,
                    first_unwritten_index: 0x1b,
                },
            ))
        })
        .unwrap_err();

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
    fn snapshot_restore_remains_valid_after_capture_inputs_are_dropped() {
        let snapshot = {
            let policy = policy(&[0x10, 0x1b]);
            let requested = [(MsrIndex::new(0x10), 11), (MsrIndex::new(0x1b), 22)];
            let values = GuestMsrValueSet::from_policy(&policy, &requested).unwrap();
            GuestMsrSnapshot::from_capture(&policy, &values).unwrap()
        };

        restore_msr_snapshot_with(&snapshot, |values| {
            assert_eq!(values.values().len(), 2);
            assert_eq!(values.values()[0].value(), 11);
            assert_eq!(values.values()[1].value(), 22);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn set_msrs_request_matches_x86_64_kvm_uapi_and_checks_capacity_before_syscall() {
        assert_eq!(sys::KVM_SET_MSRS, 0x4008_AE89);

        let mut request = sys::KvmMsrs::<1>::new();
        request.nmsrs = 2;
        let error = sys::set_msrs(-1, &request).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn empty_value_set_skips_kvm_write_request() {
        let values = value_set(&[0x10], &[]);
        assert!(prepare_vcpu_msr_write_request(&values).unwrap().is_none());
    }

    #[test]
    fn write_request_preserves_value_set_order_and_zeroes_reserved_fields() {
        let values = value_set(
            &[0x10, 0x1b, 0xc000_0080],
            &[
                (0xc000_0080, 0xaaaa_bbbb_cccc_dddd),
                (0x10, 0x1111_2222_3333_4444),
            ],
        );

        let request = prepare_vcpu_msr_write_request(&values).unwrap().unwrap();

        assert_eq!(request.nmsrs, 2);
        assert_eq!(request.pad, 0);
        assert_eq!(
            request.entries[0],
            sys::KvmMsrEntry {
                index: 0xc000_0080,
                reserved: 0,
                data: 0xaaaa_bbbb_cccc_dddd,
            }
        );
        assert_eq!(
            request.entries[1],
            sys::KvmMsrEntry {
                index: 0x10,
                reserved: 0,
                data: 0x1111_2222_3333_4444,
            }
        );
        assert_eq!(request.entries[2], sys::KvmMsrEntry::ZERO);
    }

    #[test]
    fn write_request_rejects_value_set_above_project_bound() {
        let indices: Vec<u32> = (0..=VCPU_MSR_WRITE_CAPACITY as u32).collect();
        let values: Vec<(u32, u64)> = indices
            .iter()
            .copied()
            .map(|index| (index, u64::from(index)))
            .collect();
        let values = value_set(&indices, &values);

        let error = prepare_vcpu_msr_write_request(&values).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn completion_accepts_exact_processed_count() {
        let values = value_set(&[0x10, 0x1b], &[(0x10, 1), (0x1b, 2)]);
        validate_vcpu_msr_write_completion(VcpuId::new(3), &values, 2).unwrap();
    }

    #[test]
    fn partial_completion_reports_processed_count_and_first_unwritten_index() {
        let values = value_set(&[0x10, 0x1b], &[(0x10, 1), (0x1b, 2)]);

        assert!(matches!(
            validate_vcpu_msr_write_completion(VcpuId::new(3), &values, 1),
            Err(Error::HostEnvironment(
                HostEnvironmentError::VcpuMsrPartialWrite {
                    id: 3,
                    requested: 2,
                    processed: 1,
                    first_unwritten_index: 0x1b,
                }
            ))
        ));
    }

    #[test]
    fn zero_completion_reports_first_value_as_unwritten() {
        let values = value_set(&[0x10], &[(0x10, 1)]);

        assert!(matches!(
            validate_vcpu_msr_write_completion(VcpuId::BOOT, &values, 0),
            Err(Error::HostEnvironment(
                HostEnvironmentError::VcpuMsrPartialWrite {
                    id: 0,
                    requested: 1,
                    processed: 0,
                    first_unwritten_index: 0x10,
                }
            ))
        ));
    }

    #[test]
    fn completion_rejects_processed_count_above_request() {
        let values = value_set(&[0x10], &[(0x10, 1)]);

        assert!(matches!(
            validate_vcpu_msr_write_completion(VcpuId::new(4), &values, 2),
            Err(Error::HostEnvironment(
                HostEnvironmentError::VcpuMsrInvalidWriteCompletion {
                    id: 4,
                    requested: 1,
                    processed: 2,
                }
            ))
        ));
    }
}

#[cfg(test)]
#[path = "msr_restore_verify_tests.rs"]
mod msr_restore_verify_tests;
