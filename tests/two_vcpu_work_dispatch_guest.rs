use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::two_vcpu_work_dispatch_fixture::{
    run_two_vcpu_work_dispatch, AP_TERMINAL_RIP, AP_WORK_PROOF, BSP_TERMINAL_RIP, BSP_WORK_PROOF,
    WORK_PAYLOAD, WORK_RESULT,
};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

#[test]
fn two_long_mode_vcpus_exchange_one_bounded_work_item_through_guest_ram() {
    match run_two_vcpu_work_dispatch() {
        Ok(result) => {
            assert_eq!(result.bsp_proof(), BSP_WORK_PROOF);
            assert_eq!(result.ap_proof(), AP_WORK_PROOF);
            assert_eq!(result.bsp_io_exits().len(), BSP_WORK_PROOF.len());
            assert_eq!(result.ap_io_exits().len(), AP_WORK_PROOF.len());

            for (io, expected) in result
                .bsp_io_exits()
                .iter()
                .zip(BSP_WORK_PROOF.iter().copied())
                .chain(
                    result
                        .ap_io_exits()
                        .iter()
                        .zip(AP_WORK_PROOF.iter().copied()),
                )
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            let mailbox = result.mailbox();
            assert_eq!(mailbox.payload(), WORK_PAYLOAD);
            assert_eq!(mailbox.command(), 0);
            assert_eq!(mailbox.result(), WORK_RESULT);
            assert_eq!(mailbox.ack(), 0);

            let bsp = result.bsp_report();
            assert_eq!(bsp.exit(), VcpuExit::Hlt);
            assert_eq!(bsp.rip(), BSP_TERMINAL_RIP);
            assert_eq!(bsp.rflags() & 0x2, 0x2);

            let ap = result.ap_report();
            assert_eq!(ap.exit(), VcpuExit::Hlt);
            assert_eq!(ap.rip(), AP_TERMINAL_RIP);
            assert_eq!(ap.rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping two-vCPU work-dispatch integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("two-vCPU work-dispatch guest execution failed unexpectedly: {error}"),
    }
}
