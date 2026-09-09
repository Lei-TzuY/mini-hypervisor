use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::state_snapshot::{
    run_two_vcpu_checkpoint_guest, TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP,
    TWO_VCPU_CHECKPOINT_FIRST_ID, TWO_VCPU_CHECKPOINT_FIRST_PROOF,
    TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE, TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP,
    TWO_VCPU_CHECKPOINT_OWNERSHIP_SET, TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP,
    TWO_VCPU_CHECKPOINT_SECOND_ID, TWO_VCPU_CHECKPOINT_SECOND_PROOF,
    TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE, TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP,
    TWO_VCPU_CHECKPOINT_SHARED_PAGE,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

#[test]
fn coordinated_checkpoint_restores_two_vcpus_and_shared_pages_before_resume() {
    match run_two_vcpu_checkpoint_guest() {
        Ok(result) => {
            assert_eq!(result.first_capture().vcpu_id(), TWO_VCPU_CHECKPOINT_FIRST_ID);
            assert_eq!(result.first_capture().exit(), VcpuExit::Hlt);
            assert_eq!(result.first_capture().rip(), TWO_VCPU_CHECKPOINT_FIRST_CAPTURE_RIP);
            assert_eq!(result.first_capture().rflags() & 0x2, 0x2);
            assert_eq!(result.second_capture().vcpu_id(), TWO_VCPU_CHECKPOINT_SECOND_ID);
            assert_eq!(result.second_capture().exit(), VcpuExit::Hlt);
            assert_eq!(
                result.second_capture().rip(),
                TWO_VCPU_CHECKPOINT_SECOND_CAPTURE_RIP
            );
            assert_eq!(result.second_capture().rflags() & 0x2, 0x2);

            assert_eq!(result.captured_pages(), TWO_VCPU_CHECKPOINT_OWNERSHIP_SET);
            for page in TWO_VCPU_CHECKPOINT_OWNERSHIP_SET {
                assert_eq!(result.corruption().page_exact(page), Some(false));
                assert_eq!(result.restored().page_exact(page), Some(true));
            }
            assert_eq!(
                result.corruption().vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                Some(false)
            );
            assert_eq!(
                result.corruption().vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                Some(false)
            );
            assert_eq!(
                result.restored().vcpu_exact(TWO_VCPU_CHECKPOINT_FIRST_ID),
                Some(true)
            );
            assert_eq!(
                result.restored().vcpu_exact(TWO_VCPU_CHECKPOINT_SECOND_ID),
                Some(true)
            );
            assert!(result.restored().is_exact_match());

            assert_eq!(result.first_proof(), TWO_VCPU_CHECKPOINT_FIRST_PROOF);
            assert_eq!(result.second_proof(), TWO_VCPU_CHECKPOINT_SECOND_PROOF);
            assert_eq!(result.first_io_exits().len(), 1);
            assert_eq!(result.second_io_exits().len(), 1);
            for (io, expected) in [
                (&result.first_io_exits()[0], TWO_VCPU_CHECKPOINT_FIRST_PROOF[0]),
                (&result.second_io_exits()[0], TWO_VCPU_CHECKPOINT_SECOND_PROOF[0]),
            ] {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.size(), 1);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            assert_eq!(result.first_terminal().vcpu_id(), TWO_VCPU_CHECKPOINT_FIRST_ID);
            assert_eq!(result.first_terminal().exit(), VcpuExit::Hlt);
            assert_eq!(
                result.first_terminal().rip(),
                TWO_VCPU_CHECKPOINT_FIRST_TERMINAL_RIP
            );
            assert_eq!(result.first_terminal().rflags() & 0x2, 0x2);
            assert_eq!(result.second_terminal().vcpu_id(), TWO_VCPU_CHECKPOINT_SECOND_ID);
            assert_eq!(result.second_terminal().exit(), VcpuExit::Hlt);
            assert_eq!(
                result.second_terminal().rip(),
                TWO_VCPU_CHECKPOINT_SECOND_TERMINAL_RIP
            );
            assert_eq!(result.second_terminal().rflags() & 0x2, 0x2);

            assert_eq!(TWO_VCPU_CHECKPOINT_SHARED_PAGE.get(), 0x30000);
            assert_eq!(TWO_VCPU_CHECKPOINT_FIRST_STACK_PAGE.get(), 0x1fd000);
            assert_eq!(TWO_VCPU_CHECKPOINT_SECOND_STACK_PAGE.get(), 0x1fc000);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping two-vCPU checkpoint integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("two-vCPU checkpoint guest execution failed unexpectedly: {error}"),
    }
}
