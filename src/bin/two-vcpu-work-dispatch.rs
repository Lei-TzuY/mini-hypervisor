use mini_hypervisor::portio::two_vcpu_work_dispatch_fixture::{
    run_two_vcpu_work_dispatch, AP_TERMINAL_RIP, AP_WORK_PROOF, BSP_TERMINAL_RIP, BSP_WORK_PROOF,
    WORK_PAYLOAD, WORK_RESULT,
};
use mini_hypervisor::vcpu::VcpuExit;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_work_dispatch() {
        Ok(result) => {
            let mailbox = result.mailbox();
            println!("work-dispatch BSP proof: {:?}", result.bsp_proof());
            println!("work-dispatch AP proof: {:?}", result.ap_proof());
            println!("work-dispatch payload: {:#x}", mailbox.payload());
            println!("work-dispatch command: {:#x}", mailbox.command());
            println!("work-dispatch result: {:#x}", mailbox.result());
            println!("work-dispatch ack: {:#x}", mailbox.ack());
            println!("work-dispatch BSP report: {}", result.bsp_report());
            println!("work-dispatch AP report: {}", result.ap_report());

            let valid = result.bsp_proof() == BSP_WORK_PROOF
                && result.ap_proof() == AP_WORK_PROOF
                && mailbox.payload() == WORK_PAYLOAD
                && mailbox.command() == 0
                && mailbox.result() == WORK_RESULT
                && mailbox.ack() == 0
                && result.bsp_report().exit() == VcpuExit::Hlt
                && result.bsp_report().rip() == BSP_TERMINAL_RIP
                && result.ap_report().exit() == VcpuExit::Hlt
                && result.ap_report().rip() == AP_TERMINAL_RIP;

            if valid {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: two-vCPU shared work-dispatch proof violated its fixed contract");
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
