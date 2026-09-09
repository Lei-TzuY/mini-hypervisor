use mini_hypervisor::config::VmConfig;
use mini_hypervisor::task::run_timer_task_preemption_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_timer_task_preemption_guest(VmConfig::default()) {
        Ok(result) => {
            let a = result.task_a();
            let b = result.task_b();
            let terminal = result.terminal();
            println!("timer task GSI: {}", result.gsi());
            println!("timer task vector: {:#x}", result.vector());
            println!("timer task LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("timer task LAPIC LINT0: {:#x}", result.lapic_lint0());
            println!("timer task armed rflags: {:#x}", result.armed_rflags());
            println!(
                "timer task A context: cr3={:#x} rip={:#x} rsp={:#x} rflags={:#x} r12={:#x} saves={}",
                a.cr3(),
                a.rip(),
                a.rsp(),
                a.rflags(),
                a.r12(),
                a.save_count()
            );
            println!(
                "timer task B context: cr3={:#x} rip={:#x} rsp={:#x} rflags={:#x} r12={:#x} saves={}",
                b.cr3(),
                b.rip(),
                b.rsp(),
                b.rflags(),
                b.r12(),
                b.save_count()
            );
            println!(
                "timer task terminal: cr3={:#x} rip={:#x} rsp={:#x} r12={:#x}",
                terminal.cr3(),
                terminal.rip(),
                terminal.rsp(),
                terminal.r12()
            );
            println!(
                "timer task final registers: cr3={:#x} r12={:#x}",
                result.final_cr3(),
                result.final_r12()
            );
            println!(
                "timer task stack markers: first={} second={}",
                result.task_a_stack_marker(),
                result.task_b_stack_marker()
            );
            println!("timer task A context PTE: {:#x}", result.first_context_pte());
            println!("timer task B context PTE: {:#x}", result.second_context_pte());
            println!("timer task proof: {:?}", result.proof());
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
