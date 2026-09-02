use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::run_debug_port_guest;
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit, VcpuId};

#[test]
fn deterministic_debug_port_guest_services_io_then_halts() {
    match run_debug_port_guest(VmConfig::default()) {
        Ok(result) => {
            let io = result.io();
            assert_eq!(io.direction(), PortIoDirection::Out);
            assert_eq!(io.size(), 1);
            assert_eq!(io.port(), DEBUG_PORT);
            assert_eq!(io.count(), 1);
            assert_eq!(io.output_data(), b"K");
            assert_eq!(result.output(), b"K");

            let report = result.report();
            assert_eq!(report.vcpu_id(), VcpuId::BOOT);
            assert_eq!(report.exit(), VcpuExit::Hlt);
            assert_eq!(report.rip(), 0x1005);
            assert_eq!(report.rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping debug-port guest integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("debug-port guest execution failed unexpectedly: {error}"),
    }
}
