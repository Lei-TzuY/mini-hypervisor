use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::kvm::msr::MsrIndex;
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::vcpu::VcpuId;
use std::io;

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
fn vcpu_msr_readback_is_bounded_owned_and_preserves_caller_order_when_kvm_is_available() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let vm = backend.create_vm().expect("VM creation should succeed");
    let vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("vCPU creation should succeed");

    let empty = vcpu.msrs(&[]).expect("empty MSR request should succeed");
    assert!(empty.values().is_empty());

    let requested: Vec<MsrIndex> = backend
        .host_msr_indices()
        .indices()
        .iter()
        .copied()
        .take(2)
        .collect();
    assert!(!requested.is_empty());

    let values = vcpu
        .msrs(&requested)
        .expect("supported caller-selected vCPU MSRs should be readable");
    assert_eq!(values.values().len(), requested.len());
    for (value, expected_index) in values.values().iter().zip(requested.iter().copied()) {
        assert_eq!(value.index(), expected_index);
    }

    let oversized = vec![MsrIndex::new(0); 1025];
    match vcpu.msrs(&oversized) {
        Err(Error::HostEnvironment(HostEnvironmentError::VcpuOperation {
            id,
            operation: "validate KVM_GET_MSRS request",
            source,
        })) => {
            assert_eq!(id, VcpuId::BOOT.get());
            assert_eq!(source.kind(), io::ErrorKind::InvalidInput);
        }
        Err(error) => panic!("oversized MSR request returned the wrong error: {error}"),
        Ok(_) => panic!("oversized MSR request unexpectedly succeeded"),
    }
}
