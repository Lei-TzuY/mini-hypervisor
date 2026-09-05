use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::two_vcpu_fixture::{
    run_two_vcpu_guest, FIRST_PROOF, FIRST_TERMINAL_RIP, FIRST_VCPU_ID, SECOND_PROOF,
    SECOND_TERMINAL_RIP, SECOND_VCPU_ID, SHARED_MARKER_VALUE,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

#[test]
fn two_vcpus_share_ram_but_keep_distinct_execution_contexts() {
    match run_two_vcpu_guest() {
        Ok(result) => {
            assert_eq!(result.first_proof(), FIRST_PROOF);
            assert_eq!(result.second_proof(), SECOND_PROOF);
            assert_eq!(result.shared_marker(), SHARED_MARKER_VALUE);

            assert_eq!(result.first_io_exits().len(), 1);
            let first_io = &result.first_io_exits()[0];
            assert_eq!(first_io.direction(), PortIoDirection::Out);
            assert_eq!(first_io.port(), DEBUG_PORT);
            assert_eq!(first_io.size(), 1);
            assert_eq!(first_io.count(), 1);
            assert_eq!(first_io.output_data(), FIRST_PROOF);

            assert_eq!(result.second_io_exits().len(), 1);
            let second_io = &result.second_io_exits()[0];
            assert_eq!(second_io.direction(), PortIoDirection::Out);
            assert_eq!(second_io.port(), DEBUG_PORT);
            assert_eq!(second_io.size(), 1);
            assert_eq!(second_io.count(), 1);
            assert_eq!(second_io.output_data(), SECOND_PROOF);

            let first = result.first_report();
            assert_eq!(first.vcpu_id(), FIRST_VCPU_ID);
            assert_eq!(first.exit(), VcpuExit::Hlt);
            assert_eq!(first.rip(), FIRST_TERMINAL_RIP);
            assert_eq!(first.rflags() & 0x2, 0x2);

            let second = result.second_report();
            assert_eq!(second.vcpu_id(), SECOND_VCPU_ID);
            assert_eq!(second.exit(), VcpuExit::Hlt);
            assert_eq!(second.rip(), SECOND_TERMINAL_RIP);
            assert_eq!(second.rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping two-vCPU integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("two-vCPU guest execution failed unexpectedly: {error}"),
    }
}
