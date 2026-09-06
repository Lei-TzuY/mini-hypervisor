use mini_hypervisor::config::VmConfig;
use mini_hypervisor::syscall::{
    run_syscall_sysret_guest, SYSCALL_LSTAR_VALUE, SYSCALL_PROOF, SYSCALL_TERMINAL_RETURN_RIP,
};
use mini_hypervisor::vcpu::VcpuExit;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_syscall_sysret_guest(VmConfig::default()) {
        Ok(result) => {
            let observation = result.observation();
            let frame = result.terminal_frame();
            println!("syscall proof: {:?}", result.proof());
            println!("syscall user selectors: {:?}", result.user_selectors());
            println!(
                "syscall entry: rcx={:#x} r11={:#x} user_rsp={:#x} kernel_rflags={:#x} cs={:#x} ss={:#x} kernel_rsp={:#x}",
                observation.user_return_rip(),
                observation.user_rflags(),
                observation.user_rsp(),
                observation.kernel_rflags(),
                observation.kernel_cs(),
                observation.kernel_ss(),
                observation.kernel_rsp()
            );
            println!(
                "sysret frame: rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
                frame.rip(),
                frame.cs(),
                frame.rflags(),
                frame.rsp(),
                frame.ss()
            );
            println!(
                "syscall MSRs: efer={:#x} star={:#x} lstar={:#x} sfmask={:#x}",
                result.efer(),
                result.star(),
                result.lstar(),
                result.sfmask()
            );
            println!("syscall user code PTE: {:#x}", result.user_code_pte());
            println!("syscall user stack PTE: {:#x}", result.user_stack_pte());
            println!("syscall handler PTE: {:#x}", result.syscall_handler_pte());
            println!(
                "syscall observation PTE: {:#x}",
                result.syscall_observation_pte()
            );
            println!(
                "syscall terminal: rsp={:#x} cs={:#x} rflags={:#x}",
                result.terminal_rsp(),
                result.terminal_cs(),
                result.terminal_rflags()
            );
            println!(
                "syscall terminal report: rip={:#x} rflags={:#x}",
                result.report().rip(),
                result.report().rflags()
            );

            let valid = result.proof() == SYSCALL_PROOF
                && result.lstar() == SYSCALL_LSTAR_VALUE
                && frame.rip() == SYSCALL_TERMINAL_RETURN_RIP
                && result.report().exit() == VcpuExit::Hlt
                && result.report().rip() == 0x1_3005;
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
