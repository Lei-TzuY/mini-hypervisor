use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::run_cpuid_guest;
use mini_hypervisor::vcpu::{VcpuExit, VcpuId};

#[test]
fn deterministic_cpuid_guest_observes_masked_cpu_policy() {
    match run_cpuid_guest(VmConfig::default()) {
        Ok(result) => {
            let report = result.report();
            assert_eq!(report.vcpu_id(), VcpuId::BOOT);
            assert_eq!(report.exit(), VcpuExit::Hlt);
            assert_eq!(report.rip(), 0x101c);
            assert_eq!(report.rflags() & 0x2, 0x2);
            assert_eq!(result.cpuid1_ecx() & ((1 << 21) | (1 << 24)), 0);
            assert_eq!(result.kvm_features_eax() & (1 << 7), 0);
            assert!(result.masked_lapic_features_clear());
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping CPUID guest integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("CPUID guest execution failed unexpectedly: {error}"),
    }
}
