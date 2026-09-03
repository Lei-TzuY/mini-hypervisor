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
fn policy_bound_vcpu_msr_capture_is_owned_and_preserves_policy_order_when_kvm_is_available() {
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
        .capture_msrs(&empty_policy)
        .expect("empty policy capture should succeed without a KVM MSR read");
    assert!(empty.values().is_empty());

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

    let captured = vcpu
        .capture_msrs(&policy)
        .expect("policy-authorized vCPU MSRs should be capturable");

    assert_eq!(captured.values().len(), requested.len());
    for (captured_value, expected_index) in captured.values().iter().zip(requested.iter().copied())
    {
        assert_eq!(captured_value.index(), expected_index);
    }
}
