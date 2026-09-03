use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::vcpu::{VcpuId, VcpuInternalError};

fn assert_optional_data_accessor(error: &VcpuInternalError) {
    let _: Option<&[u64]> = error.data();
}

#[test]
fn internal_error_exposes_capability_gated_optional_data_accessor() {
    let _ = assert_optional_data_accessor as fn(&VcpuInternalError);
}

#[test]
fn vcpu_inherits_internal_error_data_capability_from_backend() {
    let backend = match KvmBackend::open() {
        Ok(backend) => backend,
        Err(Error::HostEnvironment(
            HostEnvironmentError::KvmUnavailable { .. }
            | HostEnvironmentError::PermissionDenied { .. },
        )) => return,
        Err(error) => panic!("unexpected KVM backend failure: {error}"),
    };

    let vm = backend
        .create_vm()
        .expect("KVM backend should create a VM after capability validation");
    let vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("validated VM should create a boot vCPU");

    assert_eq!(
        vcpu.supports_internal_error_data(),
        backend.capabilities().supports_internal_error_data()
    );
}
