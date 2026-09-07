use mini_hypervisor::config::VmConfig;
use mini_hypervisor::task::run_task_context_switch_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_task_context_switch_guest(VmConfig::default()) {
        Ok(result) => {
            let a = result.task_a();
            let b = result.task_b();
            let terminal = result.terminal();
            println!(
                "task A context: cr3={:#x} rip={:#x} rsp={:#x} rflags={:#x} r12={:#x} saves={}",
                a.cr3(), a.rip(), a.rsp(), a.rflags(), a.r12(), a.save_count()
            );
            println!(
                "task B context: cr3={:#x} rip={:#x} rsp={:#x} rflags={:#x} r12={:#x} saves={}",
                b.cr3(), b.rip(), b.rsp(), b.rflags(), b.r12(), b.save_count()
            );
            println!(
                "task terminal: cr3={:#x} rip={:#x} rsp={:#x} r12={:#x}",
                terminal.cr3(), terminal.rip(), terminal.rsp(), terminal.r12()
            );
            println!(
                "task final registers: cr3={:#x} r12={:#x}",
                result.final_cr3(), result.final_r12()
            );
            println!(
                "task stack markers: first={} second={}",
                result.task_a_stack_marker(), result.task_b_stack_marker()
            );
            println!("task A context PTE: {:#x}", result.first_context_pte());
            println!("task B context PTE: {:#x}", result.second_context_pte());
            println!("task proof: {:?}", result.proof());
            println!("{}", result.report());
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
