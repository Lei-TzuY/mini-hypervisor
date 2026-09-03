use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::run_debug_port_input_guest;
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit, VcpuId};

#[test]
fn deterministic_debug_port_input_guest_consumes_response_then_halts() {
    match run_debug_port_input_guest(VmConfig::default()) {
        Ok(result) => {
            let io = result.io();
            assert_eq!(io.direction(), PortIoDirection::In);
            assert_eq!(io.size(), 1);
            assert_eq!(io.port(), DEBUG_PORT);
            assert_eq!(io.count(), 1);
            assert!(io.output_data().is_empty());
            assert_eq!(result.value(), b'R');

            let report = result.report();
            assert_eq!(report.vcpu_id(), VcpuId::BOOT);
            assert_eq!(report.exit(), VcpuExit::Hlt);
            assert_eq!(report.rip(), 0x1006);
            assert_eq!(report.rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping debug-port input guest integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("debug-port input guest execution failed unexpectedly: {error}"),
    }
}
