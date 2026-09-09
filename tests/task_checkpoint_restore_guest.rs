use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::state_snapshot::{
    run_bounded_checkpoint_guest, BOUNDED_CHECKPOINT_CAPTURE_RIP, BOUNDED_CHECKPOINT_MARKER,
    BOUNDED_CHECKPOINT_PROOF, BOUNDED_CHECKPOINT_TERMINAL_RIP,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

#[test]
fn bounded_checkpoint_restores_corrupted_page_and_vcpu_then_resumes_guest() {
    match run_bounded_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.checkpoint_report().exit(), VcpuExit::Hlt);
            assert_eq!(result.checkpoint_report().rip(), BOUNDED_CHECKPOINT_CAPTURE_RIP);
            assert_eq!(result.checkpoint_report().rflags() & 0x2, 0x2);

            assert!(!result.corruption().page_exact());
            assert!(!result.corruption().vcpu().is_exact_match());
            assert!(!result.corruption().is_exact_match());

            assert!(result.restored().page_exact());
            assert!(result.restored().vcpu().is_exact_match());
            assert!(result.restored().is_exact_match());
            assert_eq!(result.restored_marker(), BOUNDED_CHECKPOINT_MARKER);

            assert_eq!(result.proof(), BOUNDED_CHECKPOINT_PROOF);
            assert_eq!(result.io_exits().len(), 1);
            let io = &result.io_exits()[0];
            assert_eq!(io.direction(), PortIoDirection::Out);
            assert_eq!(io.port(), DEBUG_PORT);
            assert_eq!(io.size(), 1);
            assert_eq!(io.count(), 1);
            assert_eq!(io.output_data(), BOUNDED_CHECKPOINT_PROOF);

            assert_eq!(result.terminal_report().exit(), VcpuExit::Hlt);
            assert_eq!(result.terminal_report().rip(), BOUNDED_CHECKPOINT_TERMINAL_RIP);
            assert_eq!(result.terminal_report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!("skipping bounded checkpoint restore integration assertion: /dev/kvm is unavailable to this runner");
        }
        Err(error) => panic!("bounded checkpoint restore failed unexpectedly: {error}"),
    }
}
