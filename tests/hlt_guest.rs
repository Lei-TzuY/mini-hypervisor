use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::run_hlt_guest;
use mini_hypervisor::vcpu::VcpuExit;

#[test]
fn deterministic_hlt_guest_exits_and_advances_rip() {
    match run_hlt_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.exit, VcpuExit::Hlt);
            assert_eq!(result.rip, 0x1001);
            assert_eq!(result.rflags & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping HLT guest integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("HLT guest execution failed unexpectedly: {error}"),
    }
}
