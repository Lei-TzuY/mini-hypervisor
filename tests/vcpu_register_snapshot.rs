use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::memory::GuestPhysAddr;
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
fn vcpu_register_snapshot_is_owned_and_captures_initialized_general_register_state() {
    let Some(backend) = backend_or_skip() else {
        return;
    };
    let vm = backend.create_vm().expect("VM creation should succeed");
    let vcpu = vm
        .create_vcpu(VcpuId::BOOT)
        .expect("vCPU creation should succeed");

    let entry = GuestPhysAddr::new(0x1000);
    vcpu.initialize_real_mode(entry)
        .expect("real-mode register initialization should succeed");

    let snapshot = vcpu
        .capture_register_snapshot()
        .expect("general-register snapshot capture should succeed");
    let diagnostics = vcpu
        .registers()
        .expect("existing register diagnostics should still succeed");

    assert_eq!(snapshot.rip(), entry.get());
    assert_eq!(snapshot.rflags(), 2);
    assert_eq!(snapshot.rax(), 0);
    assert_eq!(snapshot.rsp(), 0);
    assert_eq!(snapshot.rip(), diagnostics.rip);
    assert_eq!(snapshot.rflags(), diagnostics.rflags);
}
