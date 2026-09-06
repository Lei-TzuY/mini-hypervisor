use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::privilege::{
    PRIVILEGE_TSS_RSP0, PRIVILEGE_USER_CODE_SELECTOR, PRIVILEGE_USER_DATA_SELECTOR,
    PRIVILEGE_USER_STACK,
};
use mini_hypervisor::syscall::{
    run_syscall_sysret_guest, EFER_SYSCALL_ENABLE, SYSCALL_KERNEL_STACK, SYSCALL_LSTAR_VALUE,
    SYSCALL_PROOF, SYSCALL_SFMASK_VALUE, SYSCALL_STAR_VALUE, SYSCALL_TERMINAL_RETURN_RIP,
    SYSCALL_USER_RETURN_RIP,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

const X86_PAGE_USER: u64 = 1 << 2;

#[test]
fn syscall_enters_kernel_on_manual_stack_and_sysret_returns_to_ring3() {
    match run_syscall_sysret_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.proof(), SYSCALL_PROOF);
            assert_eq!(result.io_exits().len(), 2);
            for (io, expected) in result.io_exits().iter().zip(SYSCALL_PROOF.iter().copied()) {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.size(), 1);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            assert_eq!(
                result.user_selectors(),
                [
                    PRIVILEGE_USER_CODE_SELECTOR,
                    PRIVILEGE_USER_DATA_SELECTOR,
                    PRIVILEGE_USER_CODE_SELECTOR,
                    PRIVILEGE_USER_DATA_SELECTOR,
                ]
            );
            let observation = result.observation();
            assert_eq!(observation.user_return_rip(), SYSCALL_USER_RETURN_RIP);
            assert_eq!(observation.user_rflags(), 0x202);
            assert_eq!(observation.user_rsp(), PRIVILEGE_USER_STACK);
            assert_eq!(observation.kernel_rflags(), 0x2);
            assert_eq!(observation.kernel_cs(), 0x08);
            assert_eq!(observation.kernel_ss(), 0x10);
            assert_eq!(observation.kernel_rsp(), SYSCALL_KERNEL_STACK);

            let frame = result.terminal_frame();
            assert_eq!(frame.rip(), SYSCALL_TERMINAL_RETURN_RIP);
            assert_eq!(frame.cs(), u64::from(PRIVILEGE_USER_CODE_SELECTOR));
            assert_eq!(frame.rflags(), 0x202);
            assert_eq!(frame.rsp(), PRIVILEGE_USER_STACK);
            assert_eq!(frame.ss(), u64::from(PRIVILEGE_USER_DATA_SELECTOR));

            assert_ne!(result.efer() & EFER_SYSCALL_ENABLE, 0);
            assert_eq!(result.star(), SYSCALL_STAR_VALUE);
            assert_eq!(result.lstar(), SYSCALL_LSTAR_VALUE);
            assert_eq!(result.sfmask(), SYSCALL_SFMASK_VALUE);

            assert_ne!(result.user_code_pte() & X86_PAGE_USER, 0);
            assert_ne!(result.user_stack_pte() & X86_PAGE_USER, 0);
            assert_eq!(result.syscall_handler_pte() & X86_PAGE_USER, 0);
            assert_eq!(result.syscall_observation_pte() & X86_PAGE_USER, 0);

            assert_eq!(result.terminal_rsp(), PRIVILEGE_TSS_RSP0 - 40);
            assert_eq!(result.terminal_cs(), 0x08);
            assert_eq!(result.terminal_rflags() & 0x2, 0x2);
            assert_eq!(result.terminal_rflags() & 0x200, 0);
            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rip(), 0x1_3005);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping syscall/SYSRET integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("syscall/SYSRET guest execution failed unexpectedly: {error}"),
    }
}
