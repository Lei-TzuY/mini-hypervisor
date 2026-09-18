use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::state_snapshot::{
    run_versioned_page_vcpu_checkpoint_guest, MULTI_PAGE_CHECKPOINT_CAPTURE_RIP,
    MULTI_PAGE_CHECKPOINT_CONTROL_PAGE, MULTI_PAGE_CHECKPOINT_DATA_PAGE,
    MULTI_PAGE_CHECKPOINT_OWNERSHIP_SET, MULTI_PAGE_CHECKPOINT_PROOF,
    MULTI_PAGE_CHECKPOINT_STACK_PAGE, MULTI_PAGE_CHECKPOINT_TERMINAL_RIP,
    VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT, VERSIONED_PAGE_VCPU_CHECKPOINT_VERSION,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

#[test]
fn encoded_checkpoint_materializes_restores_and_resumes_abcr() {
    match run_versioned_page_vcpu_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(
                result.schema_version(),
                VERSIONED_PAGE_VCPU_CHECKPOINT_VERSION
            );
            assert!(result.encoded_len() > 3 * 4096);
            assert!(result.msr_count() <= VERSIONED_PAGE_VCPU_CHECKPOINT_MSR_LIMIT);

            let checkpoint = result.checkpoint();
            let capture = checkpoint.checkpoint_report();
            assert_eq!(capture.exit(), VcpuExit::Hlt);
            assert_eq!(capture.rip(), MULTI_PAGE_CHECKPOINT_CAPTURE_RIP);
            assert_eq!(capture.rflags() & 0x2, 0x2);
            assert_eq!(
                checkpoint.captured_pages(),
                &MULTI_PAGE_CHECKPOINT_OWNERSHIP_SET
            );

            for address in [
                MULTI_PAGE_CHECKPOINT_CONTROL_PAGE,
                MULTI_PAGE_CHECKPOINT_DATA_PAGE,
                MULTI_PAGE_CHECKPOINT_STACK_PAGE,
            ] {
                assert_eq!(checkpoint.corruption().page_exact(address), Some(false));
                assert_eq!(checkpoint.restored().page_exact(address), Some(true));
            }
            assert!(!checkpoint.corruption().vcpu().is_exact_match());
            assert!(checkpoint.restored().is_exact_match());

            assert_eq!(checkpoint.proof(), MULTI_PAGE_CHECKPOINT_PROOF);
            assert_eq!(
                checkpoint.io_exits().len(),
                MULTI_PAGE_CHECKPOINT_PROOF.len()
            );
            for (io, expected) in checkpoint
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

            let terminal = checkpoint.terminal_report();
            assert_eq!(terminal.exit(), VcpuExit::Hlt);
            assert_eq!(terminal.rip(), MULTI_PAGE_CHECKPOINT_TERMINAL_RIP);
            assert_eq!(terminal.rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping versioned checkpoint integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("versioned checkpoint execution failed unexpectedly: {error}"),
    }
}
