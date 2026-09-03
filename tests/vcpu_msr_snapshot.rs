use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::kvm::msr::{GuestMsrAccessPolicy, MsrIndex};
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::vcpu::VcpuId;

fn backend_or_skip() -> Option<KvmBackend> {
    match KvmBackend::open() {
        Ok(backend) => Some(backend),
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping KVM integration assertion: /dev/kvm is unavailable to this runner");
            None
        }
        Err(error) => panic!("KVM backend initialization failed unexpectedly: {error}"),
    }
}

#[test]
fn full_policy_msr_snapshot_is_owned_and_preserves_policy_order_when_kvm_is_available() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let vm = backend.create_vm().expect("VM creation should succeed");
    let vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("vCPU creation should succeed");

    let empty_policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &[])
        .expect("empty guest MSR policy should be valid");
    let empty = vcpu
        .capture_msr_snapshot(&empty_policy)
        .expect("empty full-policy snapshot should succeed");
    assert!(empty.policy().entries().is_empty());
    assert!(empty.values().values().is_empty());

    let requested: Vec<MsrIndex> = backend
        .host_msr_indices()
        .indices()
        .iter()
        .copied()
        .take(2)
        .collect();
    assert!(!requested.is_empty());
    let policy = GuestMsrAccessPolicy::from_host(backend.host_msr_indices(), &requested)
        .expect("supported caller-selected MSRs should form a policy");

    let snapshot = vcpu
        .capture_msr_snapshot(&policy)
        .expect("full policy-authorized vCPU MSRs should be capturable as a snapshot");
    drop(policy);

    assert_eq!(snapshot.policy().entries().len(), requested.len());
    assert_eq!(snapshot.values().values().len(), requested.len());
    for ((policy_entry, value), expected_index) in snapshot
        .policy()
        .entries()
        .iter()
        .zip(snapshot.values().values())
        .zip(requested.iter().copied())
    {
        assert_eq!(policy_entry.index(), expected_index);
        assert_eq!(value.index(), expected_index);
    }
}
