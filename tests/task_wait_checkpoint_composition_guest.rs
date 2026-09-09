use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::task::{
    run_bounded_wait_channel_checkpoint_guest, RunnableTaskId, TaskRunState, TASK_WAIT_CHANNEL_A,
    TASK_WAIT_CHANNEL_NONE, TASK_WAIT_CHANNEL_PROOF, TASK_WAIT_CHECKPOINT_CAPTURE_RIP,
    TASK_WAIT_WRONG_CHANNEL,
};
use mini_hypervisor::vcpu::VcpuExit;

#[test]
fn wait_ownership_checkpoint_restores_machine_and_typed_scheduler_state() {
    match run_bounded_wait_channel_checkpoint_guest(VmConfig::default()) {
        Ok(result) => {
            let capture = result.checkpoint_report();
            assert_eq!(capture.exit(), VcpuExit::Hlt);
            assert_eq!(capture.rip(), TASK_WAIT_CHECKPOINT_CAPTURE_RIP);
            assert_eq!(capture.rflags() & 0x2, 0x2);

            let captured = result.captured();
            let wait = captured.wait();
            assert_eq!(wait.task_a_state(), TaskRunState::Blocked);
            assert_eq!(wait.owner(), TASK_WAIT_CHANNEL_A);
            assert_eq!(wait.mismatch_count(), 1);
            assert_eq!(wait.wake_count(), 0);
            assert_eq!(wait.last_attempt(), TASK_WAIT_WRONG_CHANNEL);
            assert_eq!(wait, result.guest().mismatch_wait());

            let queue = captured.queue();
            assert_eq!(queue.selected(), RunnableTaskId::B);
            assert_eq!(queue.task_a_state(), TaskRunState::Blocked);
            assert_eq!(queue.task_b_state(), TaskRunState::Runnable);
            assert_eq!(queue, result.guest().first_selection());

            assert!(!result.corruption().machine().page_exact());
            assert!(!result.corruption().machine().vcpu().is_exact_match());
            assert!(!result.corruption().ownership_exact());
            assert!(!result.corruption().wait_exact());
            assert!(!result.corruption().queue_exact());
            assert!(result.corruption().task_a_exact());
            assert!(result.corruption().task_b_exact());

            assert!(result.restored().machine().page_exact());
            assert!(result.restored().machine().vcpu().is_exact_match());
            assert!(result.restored().ownership_exact());
            assert!(result.restored().is_exact_match());

            assert_eq!(result.guest().proof(), TASK_WAIT_CHANNEL_PROOF);
            assert_eq!(
                result.guest().final_wait().task_a_state(),
                TaskRunState::Runnable
            );
            assert_eq!(result.guest().final_wait().owner(), TASK_WAIT_CHANNEL_NONE);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping wait checkpoint composition integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("wait checkpoint composition failed unexpectedly: {error}"),
    }
}
