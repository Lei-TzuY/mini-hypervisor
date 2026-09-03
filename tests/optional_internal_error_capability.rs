use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::kvm::{Capability, HostCapabilities, KvmBackend};

const KVM_API_VERSION: i32 = 12;
const KVM_CAP_USER_MEMORY: i32 = 3;
const KVM_CAP_SET_TSS_ADDR: i32 = 4;
const KVM_CAP_EXT_CPUID: i32 = 7;
const KVM_CAP_SET_IDENTITY_MAP_ADDR: i32 = 37;
const KVM_CAP_GET_MSR_FEATURES: i32 = 153;
const KVM_CAP_INTERNAL_ERROR_DATA: i32 = 40;

fn required_capabilities() -> Vec<Capability> {
    [
        ("KVM_CAP_USER_MEMORY", KVM_CAP_USER_MEMORY),
        ("KVM_CAP_SET_TSS_ADDR", KVM_CAP_SET_TSS_ADDR),
        ("KVM_CAP_EXT_CPUID", KVM_CAP_EXT_CPUID),
        (
            "KVM_CAP_SET_IDENTITY_MAP_ADDR",
            KVM_CAP_SET_IDENTITY_MAP_ADDR,
        ),
        ("KVM_CAP_GET_MSR_FEATURES", KVM_CAP_GET_MSR_FEATURES),
    ]
    .into_iter()
    .map(|(name, id)| Capability { name, id, value: 1 })
    .collect()
}

#[test]
fn optional_internal_error_data_support_does_not_become_required() {
    let mut capabilities = HostCapabilities {
        api_version: KVM_API_VERSION,
        vcpu_mmap_size: 4096,
        extensions: required_capabilities(),
    };

    assert!(capabilities.validate().is_ok());
    assert_eq!(capabilities.internal_error_data_capability(), None);
    assert!(!capabilities.supports_internal_error_data());

    capabilities.extensions.push(Capability {
        name: "KVM_CAP_INTERNAL_ERROR_DATA",
        id: KVM_CAP_INTERNAL_ERROR_DATA,
        value: 0,
    });
    assert!(capabilities.validate().is_ok());
    assert_eq!(
        capabilities.internal_error_data_capability(),
        Some(Capability {
            name: "KVM_CAP_INTERNAL_ERROR_DATA",
            id: KVM_CAP_INTERNAL_ERROR_DATA,
            value: 0,
        })
    );
    assert!(!capabilities.supports_internal_error_data());

    capabilities.extensions.last_mut().unwrap().value = 1;
    assert!(capabilities.validate().is_ok());
    assert!(capabilities.supports_internal_error_data());
}

#[test]
fn backend_records_optional_internal_error_data_observation_when_kvm_is_available() {
    let backend = match KvmBackend::open() {
        Ok(backend) => backend,
        Err(Error::HostEnvironment(
            HostEnvironmentError::KvmUnavailable { .. }
            | HostEnvironmentError::PermissionDenied { .. },
        )) => return,
        Err(error) => panic!("unexpected KVM backend failure: {error}"),
    };

    let capability = backend
        .capabilities()
        .internal_error_data_capability()
        .expect("backend must record the optional KVM_CAP_INTERNAL_ERROR_DATA observation");
    assert_eq!(capability.name, "KVM_CAP_INTERNAL_ERROR_DATA");
    assert_eq!(capability.id, KVM_CAP_INTERNAL_ERROR_DATA);
    assert_eq!(
        backend.capabilities().supports_internal_error_data(),
        capability.value > 0
    );
}
