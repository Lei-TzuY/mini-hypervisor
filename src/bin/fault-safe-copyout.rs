use mini_hypervisor::config::VmConfig;
use mini_hypervisor::copyout::{
    run_fault_safe_copyout_guest, COPYOUT_BAD_POINTER, COPYOUT_EFAULT, COPYOUT_PAGE_FAULT_HANDLER,
    COPYOUT_PROOF, COPYOUT_TERMINAL_HLT_RIP, COPYOUT_TERMINAL_RETURN_RIP, COPYOUT_VALUE,
};
use mini_hypervisor::vcpu::VcpuExit;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_fault_safe_copyout_guest(VmConfig::default()) {
        Ok(result) => {
            let fault = result.page_fault();
            let frame = result.terminal_frame();
            println!("copyout proof: {:?}", result.proof());
            println!(
                "copyout results: good={:#x} bad={:#x} readback={:#x} memory={:#x}",
                result.good_return(),
                result.bad_return(),
                result.user_readback(),
                result.user_memory_value()
            );
            println!(
                "copyout page fault: cr2={:#x} error={:#x} rip={:#x} cs={:#x} rflags={:#x}",
                fault.cr2(),
                fault.error_code(),
                fault.rip(),
                fault.cs(),
                fault.rflags()
            );
            println!(
                "copyout terminal frame: rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
                frame.rip(),
                frame.cs(),
                frame.rflags(),
                frame.rsp(),
                frame.ss()
            );
            println!(
                "copyout terminal: rsp={:#x} cs={:#x} rflags={:#x} cr2={:#x}",
                result.terminal_rsp(),
                result.terminal_cs(),
                result.terminal_rflags(),
                result.final_cr2()
            );
            println!(
                "copyout MSRs: efer={:#x} star={:#x} lstar={:#x} sfmask={:#x}",
                result.efer(),
                result.star(),
                result.lstar(),
                result.sfmask()
            );
            println!("copyout good page PTE: {:#x}", result.good_page_pte());
            println!(
                "copyout fault handler PTE: {:#x}",
                result.fault_handler_pte()
            );
            println!(
                "copyout fault observation PTE: {:#x}",
                result.fault_observation_pte()
            );
            println!("copyout bad PD entry: {:#x}", result.bad_pd_entry());
            println!(
                "copyout terminal report: rip={:#x} rflags={:#x}",
                result.report().rip(),
                result.report().rflags()
            );

            let valid = result.proof() == COPYOUT_PROOF
                && result.good_return() == 0
                && result.bad_return() == COPYOUT_EFAULT
                && result.user_readback() == u64::from(COPYOUT_VALUE)
                && result.user_memory_value() == COPYOUT_VALUE
                && fault.cr2() == COPYOUT_BAD_POINTER
                && fault.error_code() == 0x2
                && result.final_cr2() == COPYOUT_BAD_POINTER
                && result.good_page_pte() & 0x6 == 0x6
                && result.fault_handler_pte() & 0x4 == 0
                && COPYOUT_PAGE_FAULT_HANDLER.get() == 0x1_4000
                && frame.rip() == COPYOUT_TERMINAL_RETURN_RIP
                && result.report().exit() == VcpuExit::Hlt
                && result.report().rip() == COPYOUT_TERMINAL_HLT_RIP;
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
