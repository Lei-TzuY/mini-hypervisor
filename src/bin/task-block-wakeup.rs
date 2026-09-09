use mini_hypervisor::config::VmConfig;
use mini_hypervisor::task::run_task_block_wake_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_task_block_wake_guest(VmConfig::default()) {
        Ok(result) => {
            let a = result.task_a();
            let b = result.task_b();
            let terminal = result.terminal();
            println!("task wake GSI: {}", result.gsi());
            println!("task wake vector: {:#x}", result.vector());
            println!("task wake LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("task wake LAPIC LINT0: {:#x}", result.lapic_lint0());
            println!("task wake armed rflags: {:#x}", result.armed_rflags());
            println!(
                "task wake states: blocked={:?} wake={:?} final={:?}",
                result.blocked_state(),
                result.wake_state(),
                result.final_state()
            );
            println!(
                "task wake A context: cr3={:#x} rip={:#x} rsp={:#x} rflags={:#x} r12={:#x} saves={}",
                a.cr3(),
                a.rip(),
                a.rsp(),
                a.rflags(),
                a.r12(),
                a.save_count()
            );
            println!(
                "task wake B context: cr3={:#x} rip={:#x} rsp={:#x} rflags={:#x} r12={:#x} saves={}",
                b.cr3(),
                b.rip(),
                b.rsp(),
                b.rflags(),
                b.r12(),
                b.save_count()
            );
            println!(
                "task wake terminal: cr3={:#x} rip={:#x} rsp={:#x} r12={:#x}",
                terminal.cr3(),
                terminal.rip(),
                terminal.rsp(),
                terminal.r12()
            );
            println!(
                "task wake final registers: cr3={:#x} r12={:#x}",
                result.final_cr3(),
                result.final_r12()
            );
            println!(
                "task wake stack markers: first={} second={}",
                result.task_a_stack_marker(),
                result.task_b_stack_marker()
            );
            println!("task wake A context PTE: {:#x}", result.first_context_pte());
            println!(
                "task wake B context PTE: {:#x}",
                result.second_context_pte()
            );
            println!("task wake proof: {:?}", result.proof());
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
