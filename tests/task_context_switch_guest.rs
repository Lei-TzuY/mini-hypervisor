use mini_hypervisor::address_space::{ADDRESS_SPACE_A_CR3, ADDRESS_SPACE_B_PML4_ADDR};
use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::task::{
    run_task_context_switch_guest, TASK_A_INITIAL_RSP, TASK_A_R12, TASK_A_SAVED_RIP,
    TASK_B_INITIAL_RSP, TASK_B_SAVED_R12, TASK_B_SAVED_RIP, TASK_CONTEXT_PAGE_ADDR,
    TASK_CONTEXT_PROOF, TASK_TERMINAL_RIP, TASK_TERMINAL_USER_RIP,
};
use mini_hypervisor::vcpu::{PortIoDirection, VcpuExit};

const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITABLE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_PAGE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

#[test]
fn guest_scheduler_restores_task_address_register_and_stack_context() {
    match run_task_context_switch_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.proof(), TASK_CONTEXT_PROOF);
            assert_eq!(result.io_exits().len(), TASK_CONTEXT_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(TASK_CONTEXT_PROOF.iter().copied())
            {
                assert_eq!(io.direction(), PortIoDirection::Out);
                assert_eq!(io.port(), DEBUG_PORT);
                assert_eq!(io.size(), 1);
                assert_eq!(io.count(), 1);
                assert_eq!(io.output_data(), &[expected]);
            }

            let a = result.task_a();
            assert_eq!(a.cr3(), ADDRESS_SPACE_A_CR3.get());
            assert_eq!(a.rip(), TASK_A_SAVED_RIP);
            assert_eq!(a.rsp(), TASK_A_INITIAL_RSP);
            assert_eq!(a.rflags(), 0x202);
            assert_eq!(a.r12(), TASK_A_R12);
            assert_eq!(a.save_count(), 1);

            let b = result.task_b();
            assert_eq!(b.cr3(), ADDRESS_SPACE_B_PML4_ADDR.get());
            assert_eq!(b.rip(), TASK_B_SAVED_RIP);
            assert_eq!(b.rsp(), TASK_B_INITIAL_RSP);
            assert_eq!(b.rflags(), 0x202);
            assert_eq!(b.r12(), TASK_B_SAVED_R12);
            assert_eq!(b.save_count(), 1);

            let terminal = result.terminal();
            assert_eq!(terminal.cr3(), ADDRESS_SPACE_A_CR3.get());
            assert_eq!(terminal.rip(), TASK_TERMINAL_USER_RIP);
            assert_eq!(terminal.rsp(), TASK_A_INITIAL_RSP);
            assert_eq!(terminal.r12(), TASK_A_R12);
            assert_eq!(result.final_cr3(), ADDRESS_SPACE_A_CR3.get());
            assert_eq!(result.final_r12(), TASK_A_R12);
            assert_eq!(result.task_a_stack_marker(), b'a');
            assert_eq!(result.task_b_stack_marker(), b'b');

            for pte in [result.first_context_pte(), result.second_context_pte()] {
                assert_eq!(pte & X86_PAGE_ADDRESS_MASK, TASK_CONTEXT_PAGE_ADDR.get());
                assert_eq!(pte & X86_PAGE_PRESENT, X86_PAGE_PRESENT);
                assert_eq!(pte & X86_PAGE_WRITABLE, X86_PAGE_WRITABLE);
                assert_eq!(pte & X86_PAGE_USER, 0);
            }

            assert_eq!(result.report().exit(), VcpuExit::Hlt);
            assert_eq!(result.report().rip(), TASK_TERMINAL_RIP);
            assert_eq!(result.report().rflags() & 0x2, 0x2);
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping task context switch integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("task context switch guest execution failed unexpectedly: {error}"),
    }
}
