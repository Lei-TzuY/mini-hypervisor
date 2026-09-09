use mini_hypervisor::address_space::{ADDRESS_SPACE_A_CR3, ADDRESS_SPACE_B_PML4_ADDR};
use mini_hypervisor::config::VmConfig;
use mini_hypervisor::error::{Error, HostEnvironmentError};
use mini_hypervisor::interrupt::X86_RFLAGS_INTERRUPT_ENABLE;
use mini_hypervisor::portio::DEBUG_PORT;
use mini_hypervisor::task::{
    run_bounded_runnable_queue_guest, RunnableTaskId, TaskRunState, TASK_A_INITIAL_RSP, TASK_A_R12,
    TASK_A_SAVED_RIP, TASK_B_INITIAL_RSP, TASK_B_SAVED_R12, TASK_B_SAVED_RIP,
    TASK_CONTEXT_PAGE_ADDR, TASK_RUNNABLE_QUEUE_PROOF, TASK_TERMINAL_USER_RIP, TASK_WAKE_GSI,
    TASK_WAKE_TIMER_VECTOR,
};
use mini_hypervisor::vcpu::PortIoDirection;

const X86_PAGE_PRESENT: u64 = 1;
const X86_PAGE_WRITABLE: u64 = 1 << 1;
const X86_PAGE_USER: u64 = 1 << 2;
const X86_PAGE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const APIC_SPIV_SOFTWARE_ENABLE: u32 = 1 << 8;
const APIC_LVT_MASKED: u32 = 1 << 16;
const APIC_LVT_DELIVERY_MODE_MASK: u32 = 0x700;
const APIC_LVT_DELIVERY_MODE_EXTINT: u32 = 0x700;

#[test]
fn bounded_queue_skips_blocked_a_then_selects_woken_a() {
    match run_bounded_runnable_queue_guest(VmConfig::default()) {
        Ok(result) => {
            assert_eq!(result.gsi(), TASK_WAKE_GSI);
            assert_eq!(result.vector(), TASK_WAKE_TIMER_VECTOR);
            assert_eq!(
                result.lapic_spiv() & APIC_SPIV_SOFTWARE_ENABLE,
                APIC_SPIV_SOFTWARE_ENABLE
            );
            assert_eq!(
                result.lapic_lint0() & APIC_LVT_DELIVERY_MODE_MASK,
                APIC_LVT_DELIVERY_MODE_EXTINT
            );
            assert_eq!(result.lapic_lint0() & APIC_LVT_MASKED, 0);
            assert_eq!(result.armed_rflags() & 0x2, 0x2);
            assert_eq!(
                result.armed_rflags() & X86_RFLAGS_INTERRUPT_ENABLE,
                0,
                "P arm barrier must still have IF clear before sti;hlt"
            );

            let first = result.first_selection();
            assert_eq!(first.entry0(), RunnableTaskId::A);
            assert_eq!(first.entry1(), RunnableTaskId::B);
            assert_eq!(first.head(), 0);
            assert_eq!(first.selected(), RunnableTaskId::B);
            assert_eq!(first.skip_count(), 1);
            assert_eq!(first.task_a_state(), TaskRunState::Blocked);
            assert_eq!(first.task_b_state(), TaskRunState::Runnable);

            let second = result.second_selection();
            assert_eq!(second.entry0(), RunnableTaskId::A);
            assert_eq!(second.entry1(), RunnableTaskId::B);
            assert_eq!(second.head(), 1);
            assert_eq!(second.selected(), RunnableTaskId::A);
            assert_eq!(second.skip_count(), 1);
            assert_eq!(second.task_a_state(), TaskRunState::Runnable);
            assert_eq!(second.task_b_state(), TaskRunState::Runnable);

            assert_eq!(result.proof(), TASK_RUNNABLE_QUEUE_PROOF);
            assert_eq!(result.io_exits().len(), TASK_RUNNABLE_QUEUE_PROOF.len());
            for (io, expected) in result
                .io_exits()
                .iter()
                .zip(TASK_RUNNABLE_QUEUE_PROOF.iter().copied())
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
        }
        Err(Error::HostEnvironment(HostEnvironmentError::KvmUnavailable { .. }))
        | Err(Error::HostEnvironment(HostEnvironmentError::PermissionDenied { .. })) => {
            eprintln!(
                "skipping runnable-queue integration assertion: /dev/kvm is unavailable to this runner"
            );
        }
        Err(error) => panic!("bounded runnable-queue guest execution failed unexpectedly: {error}"),
    }
}
