use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::privilege::{
    run_privilege_transition_guest, PRIVILEGE_PROOF, PRIVILEGE_TERMINAL_RIP, PRIVILEGE_TSS_ADDR,
    PRIVILEGE_TSS_RSP0, PRIVILEGE_TSS_SELECTOR, PRIVILEGE_USER_CODE_SELECTOR,
    PRIVILEGE_USER_DATA_SELECTOR, PRIVILEGE_USER_RETURN_RIP, PRIVILEGE_USER_STACK,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

const X86_PAGE_USER: u64 = 1 << 2;
const X86_RFLAGS_RESERVED: u64 = 1 << 1;
const X86_RFLAGS_IF: u64 = 1 << 9;
const PRIVILEGE_FRAME_BYTES: u64 = 5 * 8;

#[test]
fn ring3_trap_uses_tss_rsp0_returns_to_user_and_finishes_in_ring0() {
    match run_privilege_transition_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.proof(), PRIVILEGE_PROOF);
            assert_eq!(result.io_exits().len(), 2);
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(PRIVILEGE_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.size(), 1);
                assert_eq!(io.port(), DEBUG_PORT);
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

            let frame = result.frame();
            assert_eq!(frame.rip(), PRIVILEGE_USER_RETURN_RIP);
            assert_eq!(frame.cs(), u64::from(PRIVILEGE_USER_CODE_SELECTOR));
            assert_eq!(frame.rflags(), X86_RFLAGS_RESERVED | X86_RFLAGS_IF);
            assert_eq!(frame.rsp(), PRIVILEGE_USER_STACK);
            assert_eq!(frame.ss(), u64::from(PRIVILEGE_USER_DATA_SELECTOR));

            assert_eq!(
                result.terminal_rsp(),
                PRIVILEGE_TSS_RSP0 - PRIVILEGE_FRAME_BYTES
            );
            assert_eq!(result.terminal_cs(), 0x08);
            assert_eq!(
                result.terminal_rflags() & X86_RFLAGS_RESERVED,
                X86_RFLAGS_RESERVED
            );
            assert_eq!(result.terminal_rflags() & X86_RFLAGS_IF, 0);

            assert_eq!(result.tr_selector(), PRIVILEGE_TSS_SELECTOR);
            assert_eq!(result.tr_base(), PRIVILEGE_TSS_ADDR.get());
            assert_eq!(result.tr_limit(), 103);
            assert_eq!(result.tr_type(), 0x0b);
            assert_eq!(result.tss_descriptor_access(), 0x8b);

            assert_ne!(result.user_code_pte() & X86_PAGE_USER, 0);
            assert_ne!(result.observation_pte() & X86_PAGE_USER, 0);
            assert_ne!(result.user_stack_pte() & X86_PAGE_USER, 0);
            assert_eq!(result.kernel_handler_pte() & X86_PAGE_USER, 0);

            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rip(), PRIVILEGE_TERMINAL_RIP);
            assert_eq!(
                result.report().rflags() & X86_RFLAGS_RESERVED,
                X86_RFLAGS_RESERVED
            );
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping ring3/TSS privilege integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("ring3/TSS privilege guest execution failed unexpectedly: {error}"),
    }
}
