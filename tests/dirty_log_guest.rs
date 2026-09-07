use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::kvm::KvmBackend;
use mini_hypervisor::vcpu::VcpuExit;

#[test]
fn guest_writes_only_dirty_pages_one_and_three_then_harvest_clears_them() {
    match KvmBackend::run_dirty_log_guest(VmConfig::default()) {
        Ok((first, second, values, proof, report)) => {
            assert_eq!(first, [KvmBackend::DIRTY_LOG_EXPECTED_BITMAP]);
            assert_eq!(first, [0b1010]);
            assert_eq!(second, [0]);
            assert_eq!(values, [b'A', b'B']);
            assert_eq!(proof, KvmBackend::DIRTY_LOG_PROOF);
            assert_eq!(report.exit(), VcpuExit::Hlt);
            assert_eq!(report.rip(), KvmBackend::DIRTY_LOG_TERMINAL_RIP);
            assert_eq!(report.rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping dirty-log integration assertion: /dev/kvm is unavailable to this runner");
        }
        Err(error) => panic!("dirty-log guest execution failed unexpectedly: {error}"),
    }
}
