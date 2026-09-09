use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::task::{
    run_bounded_wait_channel_dirty_guest, TaskRunState, TASK_WAIT_CHANNEL_A,
    TASK_WAIT_CHANNEL_NONE, TASK_WAIT_CHANNEL_PROOF, TASK_WAIT_DIRTY_CAPTURE_STAGES,
    TASK_WAIT_WRONG_CHANNEL,
};

#[test]
fn wait_channel_state_mutations_are_incrementally_dirty_and_cleared() {
    match run_bounded_wait_channel_dirty_guest(VmConfig::default()) {
        Ok(result) => {
            let guest = result.guest();
            assert_eq!(guest.proof(), TASK_WAIT_CHANNEL_PROOF);
            assert_eq!(result.captures().len(), TASK_WAIT_DIRTY_CAPTURE_STAGES.len());
            assert_eq!(
                result.captures().iter().map(|capture| capture.stage()).collect::<Vec<_>>(),
                TASK_WAIT_DIRTY_CAPTURE_STAGES
            );

            let expected_waits = [
                (TaskRunState::Blocked, TASK_WAIT_CHANNEL_A, 0, 0, TASK_WAIT_CHANNEL_NONE),
                (TaskRunState::Blocked, TASK_WAIT_CHANNEL_A, 1, 0, TASK_WAIT_WRONG_CHANNEL),
                (TaskRunState::Runnable, TASK_WAIT_CHANNEL_NONE, 1, 1, TASK_WAIT_CHANNEL_A),
                (TaskRunState::Runnable, TASK_WAIT_CHANNEL_NONE, 1, 1, TASK_WAIT_CHANNEL_A),
            ];

            for (capture, expected) in result.captures().iter().zip(expected_waits) {
                assert!(capture.context_page_dirty());
                assert_eq!(capture.bitmap().len(), 8);
                let wait = capture.wait();
                assert_eq!(wait.task_a_state(), expected.0);
                assert_eq!(wait.owner(), expected.1);
                assert_eq!(wait.mismatch_count(), expected.2);
                assert_eq!(wait.wake_count(), expected.3);
                assert_eq!(wait.last_attempt(), expected.4);
            }

            assert_eq!(result.captures()[0].wait(), guest.blocked_wait());
            assert_eq!(result.captures()[1].wait(), guest.mismatch_wait());
            assert_eq!(result.captures()[2].wait(), guest.wake_wait());
            assert_eq!(result.captures()[3].wait(), guest.final_wait());
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping wait dirty-capture integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("wait dirty-capture guest execution failed unexpectedly: {error}"),
    }
}
