use mini_hypervisor::config::VmConfig;
use mini_hypervisor::task::run_bounded_runnable_queue_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_bounded_runnable_queue_guest(VmConfig::default()) {
        Ok(result) => {
            let first = result.first_selection();
            let second = result.second_selection();
            let a = result.task_a();
            let b = result.task_b();
            let terminal = result.terminal();

            println!("task queue GSI: {}", result.gsi());
            println!("task queue vector: {:#x}", result.vector());
            println!("task queue LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("task queue LAPIC LINT0: {:#x}", result.lapic_lint0());
            println!("task queue armed rflags: {:#x}", result.armed_rflags());
            println!(
                "task queue first selection: entry0={:?} entry1={:?} head={} selected={:?} skips={} A={:?} B={:?}",
                first.entry0(),
                first.entry1(),
                first.head(),
                first.selected(),
                first.skip_count(),
                first.task_a_state(),
                first.task_b_state()
            );
            println!(
                "task queue second selection: entry0={:?} entry1={:?} head={} selected={:?} skips={} A={:?} B={:?}",
                second.entry0(),
                second.entry1(),
                second.head(),
                second.selected(),
                second.skip_count(),
                second.task_a_state(),
                second.task_b_state()
            );
            println!(
                "task queue A context: cr3={:#x} rip={:#x} rsp={:#x} rflags={:#x} r12={:#x} saves={}",
                a.cr3(),
                a.rip(),
                a.rsp(),
                a.rflags(),
                a.r12(),
                a.save_count()
            );
            println!(
                "task queue B context: cr3={:#x} rip={:#x} rsp={:#x} rflags={:#x} r12={:#x} saves={}",
                b.cr3(),
                b.rip(),
                b.rsp(),
                b.rflags(),
                b.r12(),
                b.save_count()
            );
            println!(
                "task queue terminal: cr3={:#x} rip={:#x} rsp={:#x} r12={:#x}",
                terminal.cr3(),
                terminal.rip(),
                terminal.rsp(),
                terminal.r12()
            );
            println!(
                "task queue final registers: cr3={:#x} r12={:#x}",
                result.final_cr3(),
                result.final_r12()
            );
            println!(
                "task queue stack markers: first={} second={}",
                result.task_a_stack_marker(),
                result.task_b_stack_marker()
            );
            println!(
                "task queue A context PTE: {:#x}",
                result.first_context_pte()
            );
            println!(
                "task queue B context PTE: {:#x}",
                result.second_context_pte()
            );
            println!("task queue proof: {:?}", result.proof());
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
