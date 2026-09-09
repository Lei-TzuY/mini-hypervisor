use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::state_snapshot::{
    run_multi_page_checkpoint_guest, MULTI_PAGE_CHECKPOINT_CAPTURE_RIP,
    MULTI_PAGE_CHECKPOINT_CONTROL_PAGE, MULTI_PAGE_CHECKPOINT_DATA_PAGE,
    MULTI_PAGE_CHECKPOINT_OWNERSHIP_SET, MULTI_PAGE_CHECKPOINT_PROOF,
    MULTI_PAGE_CHECKPOINT_STACK_PAGE, MULTI_PAGE_CHECKPOINT_TERMINAL_RIP,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

#[test]
fn three_owned_pages_and_vcpu_restore_exactly_before_abcr_resume() {
    match run_multi_page_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            let capture = result.checkpoint_report();
            assert_eq!(capture.exit(), VcpuExit::Hlt);
            assert_eq!(capture.rip(), MULTI_PAGE_CHECKPOINT_CAPTURE_RIP);
            assert_eq!(capture.rflags() & 0x2, 0x2);
            assert_eq!(result.captured_pages(), &MULTI_PAGE_CHECKPOINT_OWNERSHIP_SET);

            for address in [
                MULTI_PAGE_CHECKPOINT_CONTROL_PAGE,
                MULTI_PAGE_CHECKPOINT_DATA_PAGE,
                MULTI_PAGE_CHECKPOINT_STACK_PAGE,
            ] {
                assert_eq!(result.corruption().page_exact(address), Some(false));
                assert_eq!(result.restored().page_exact(address), Some(true));
            }
            assert!(!result.corruption().vcpu().is_exact_match());
            assert!(result.restored().vcpu().is_exact_match());
            assert!(result.restored().is_exact_match());

            assert_eq!(result.proof(), MULTI_PAGE_CHECKPOINT_PROOF);
            assert_eq!(result.io_exits().len(), MULTI_PAGE_CHECKPOINT_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(MULTI_PAGE_CHECKPOINT_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.size(), 1);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            let terminal = result.terminal_report();
            assert_eq!(terminal.exit(), VcpuExit::Hlt);
            assert_eq!(terminal.rip(), MULTI_PAGE_CHECKPOINT_TERMINAL_RIP);
            assert_eq!(terminal.rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping multi-page checkpoint integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("multi-page checkpoint execution failed unexpectedly: {error}"),
    }
}
