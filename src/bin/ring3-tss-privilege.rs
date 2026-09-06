use mini_hypervisor::config::VmConfig;
use mini_hypervisor::privilege::{
    run_privilege_transition_guest, PRIVILEGE_PROOF, PRIVILEGE_TERMINAL_RIP,
};
use mini_hypervisor::vcpu::VcpuExit;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_privilege_transition_guest(VmConfig::default()) {
        Ok(result) => {
            let frame = result.frame();
            println!("ring3 proof: {:?}", result.proof());
            println!("ring3 user selectors: {:?}", result.user_selectors());
            println!(
                "ring3 frame: rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
                frame.rip(),
                frame.cs(),
                frame.rflags(),
                frame.rsp(),
                frame.ss()
            );
            println!(
                "ring3 terminal: rsp={:#x} cs={:#x} rflags={:#x}",
                result.terminal_rsp(),
                result.terminal_cs(),
                result.terminal_rflags()
            );
            println!(
                "ring3 TR: selector={:#x} base={:#x} limit={:#x} type={:#x}",
                result.tr_selector(),
                result.tr_base(),
                result.tr_limit(),
                result.tr_type()
            );
            println!("ring3 TSS access: {:#x}", result.tss_descriptor_access());
            println!("ring3 user code PTE: {:#x}", result.user_code_pte());
            println!("ring3 observation PTE: {:#x}", result.observation_pte());
            println!("ring3 user stack PTE: {:#x}", result.user_stack_pte());
            println!(
                "ring3 kernel handler PTE: {:#x}",
                result.kernel_handler_pte()
            );
            println!(
                "ring3 terminal report: rip={:#x} rflags={:#x}",
                result.report().rip(),
                result.report().rflags()
            );

            let valid = result.proof() == PRIVILEGE_PROOF
                && result.report().exit() == VcpuExit::Hlt
                && result.report().rip() == PRIVILEGE_TERMINAL_RIP;
            if valid {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
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
