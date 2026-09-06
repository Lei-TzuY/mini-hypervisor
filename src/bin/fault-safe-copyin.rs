use mini_hypervisor::config::VmConfig;
use mini_hypervisor::copyin::{
    run_fault_safe_copyin_guest, COPYIN_BAD_POINTER, COPYIN_EFAULT, COPYIN_GOOD_VALUE,
    COPYIN_PAGE_FAULT_HANDLER, COPYIN_PROOF, COPYIN_TERMINAL_HLT_RIP, COPYIN_TERMINAL_RETURN_RIP,
};
use mini_hypervisor::vcpu::VcpuExit;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_fault_safe_copyin_guest(VmConfig::default()) {
        Ok(result) => {
            let fault = result.page_fault();
            let frame = result.terminal_frame();
            println!("copyin proof: {:?}", result.proof());
            println!(
                "copyin results: good={:#x} bad={:#x}",
                result.good_result(),
                result.bad_result()
            );
            println!(
                "copyin page fault: cr2={:#x} error={:#x} rip={:#x} cs={:#x} rflags={:#x}",
                fault.cr2(),
                fault.error_code(),
                fault.rip(),
                fault.cs(),
                fault.rflags()
            );
            println!(
                "copyin terminal frame: rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
                frame.rip(),
                frame.cs(),
                frame.rflags(),
                frame.rsp(),
                frame.ss()
            );
            println!(
                "copyin terminal: rsp={:#x} cs={:#x} rflags={:#x} cr2={:#x}",
                result.terminal_rsp(),
                result.terminal_cs(),
                result.terminal_rflags(),
                result.final_cr2()
            );
            println!(
                "copyin MSRs: efer={:#x} star={:#x} lstar={:#x} sfmask={:#x}",
                result.efer(),
                result.star(),
                result.lstar(),
                result.sfmask()
            );
            println!("copyin good page PTE: {:#x}", result.good_page_pte());
            println!(
                "copyin fault handler PTE: {:#x}",
                result.fault_handler_pte()
            );
            println!(
                "copyin fault observation PTE: {:#x}",
                result.fault_observation_pte()
            );
            println!("copyin bad PD entry: {:#x}", result.bad_pd_entry());
            println!(
                "copyin terminal report: rip={:#x} rflags={:#x}",
                result.report().rip(),
                result.report().rflags()
            );

            let valid = result.proof() == COPYIN_PROOF
                && result.good_result() == u64::from(COPYIN_GOOD_VALUE)
                && result.bad_result() == COPYIN_EFAULT
                && fault.cr2() == COPYIN_BAD_POINTER
                && result.final_cr2() == COPYIN_BAD_POINTER
                && result.fault_handler_pte() & 0x4 == 0
                && COPYIN_PAGE_FAULT_HANDLER.get() == 0x1_4000
                && frame.rip() == COPYIN_TERMINAL_RETURN_RIP
                && result.report().exit() == VcpuExit::Hlt
                && result.report().rip() == COPYIN_TERMINAL_HLT_RIP;
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
