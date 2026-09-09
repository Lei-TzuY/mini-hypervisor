use mini_hypervisor::config::VmConfig;
use mini_hypervisor::task::run_bounded_wait_channel_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_bounded_wait_channel_guest(VmConfig::default()) {
        Ok(result) => {
            let blocked = result.blocked_wait();
            let mismatch = result.mismatch_wait();
            let wake = result.wake_wait();
            let final_wait = result.final_wait();
            let first = result.first_selection();
            let second = result.second_selection();
            let a = result.task_a();
            let b = result.task_b();
            let terminal = result.terminal();

            println!("wait channel GSI: {}", result.gsi());
            println!("wait channel vector: {:#x}", result.vector());
            println!("wait channel LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("wait channel LAPIC LINT0: {:#x}", result.lapic_lint0());
            println!("wait channel armed rflags: {:#x}", result.armed_rflags());
            println!(
                "wait channel blocked: state={:?} owner={:#x} mismatches={} wakes={} last={:#x}",
                blocked.task_a_state(),
                blocked.owner(),
                blocked.mismatch_count(),
                blocked.wake_count(),
                blocked.last_attempt()
            );
            println!(
                "wait channel mismatch: state={:?} owner={:#x} mismatches={} wakes={} last={:#x}",
                mismatch.task_a_state(),
                mismatch.owner(),
                mismatch.mismatch_count(),
                mismatch.wake_count(),
                mismatch.last_attempt()
            );
            println!(
                "wait channel wake: state={:?} owner={:#x} mismatches={} wakes={} last={:#x}",
                wake.task_a_state(),
                wake.owner(),
                wake.mismatch_count(),
                wake.wake_count(),
                wake.last_attempt()
            );
            println!(
                "wait channel final: state={:?} owner={:#x} mismatches={} wakes={} last={:#x}",
                final_wait.task_a_state(),
                final_wait.owner(),
                final_wait.mismatch_count(),
                final_wait.wake_count(),
                final_wait.last_attempt()
            );
            println!(
                "wait channel first selection: entry0={:?} entry1={:?} head={} selected={:?} skips={} A={:?} B={:?}",
                first.entry0(),
                first.entry1(),
                first.head(),
                first.selected(),
                first.skip_count(),
                first.task_a_state(),
                first.task_b_state()
            );
            println!(
                "wait channel second selection: entry0={:?} entry1={:?} head={} selected={:?} skips={} A={:?} B={:?}",
                second.entry0(),
                second.entry1(),
                second.head(),
                second.selected(),
                second.skip_count(),
                second.task_a_state(),
                second.task_b_state()
            );
            println!(
                "wait channel A context: cr3={:#x} rip={:#x} rsp={:#x} rflags={:#x} r12={:#x} saves={}",
                a.cr3(),
                a.rip(),
                a.rsp(),
                a.rflags(),
                a.r12(),
                a.save_count()
            );
            println!(
                "wait channel B context: cr3={:#x} rip={:#x} rsp={:#x} rflags={:#x} r12={:#x} saves={}",
                b.cr3(),
                b.rip(),
                b.rsp(),
                b.rflags(),
                b.r12(),
                b.save_count()
            );
            println!(
                "wait channel terminal: cr3={:#x} rip={:#x} rsp={:#x} r12={:#x}",
                terminal.cr3(),
                terminal.rip(),
                terminal.rsp(),
                terminal.r12()
            );
            println!(
                "wait channel final registers: cr3={:#x} r12={:#x}",
                result.final_cr3(),
                result.final_r12()
            );
            println!(
                "wait channel stack markers: first={} second={}",
                result.task_a_stack_marker(),
                result.task_b_stack_marker()
            );
            println!(
                "wait channel A context PTE: {:#x}",
                result.first_context_pte()
            );
            println!(
                "wait channel B context PTE: {:#x}",
                result.second_context_pte()
            );
            println!("wait channel proof: {:?}", result.proof());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                eprintln!("caused by: {cause}");
                source = cause.source();
            }
            ExitCode::FAILURE
        }
    }
}
