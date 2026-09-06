use mini_hypervisor::portio::two_vcpu_sipi_work_dispatch_fixture::{
    run_sipi_ipi_work_dispatch, AP_COMPOSED_PROOF, AP_TERMINAL_RIP, BSP_COMPOSED_PROOF,
    BSP_TERMINAL_RIP,
};
use mini_hypervisor::portio::two_vcpu_work_dispatch_fixture::{WORK_PAYLOAD, WORK_RESULT};
use mini_hypervisor::vcpu::VcpuExit;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_sipi_ipi_work_dispatch() {
        Ok(result) => {
            let mailbox = result.mailbox();
            let ap = result.ap_state();
            println!("sipi-work BSP proof: {:?}", result.bsp_proof());
            println!("sipi-work AP proof: {:?}", result.ap_proof());
            println!("sipi-work payload: {:#x}", mailbox.payload());
            println!("sipi-work command: {:#x}", mailbox.command());
            println!("sipi-work result: {:#x}", mailbox.result());
            println!("sipi-work ack: {:#x}", mailbox.ack());
            println!("sipi-work initial AP MP state: {}", result.initial_ap_mp_state());
            println!(
                "sipi-work AP startup: mp={} rip={:#x} cs={:#x} base={:#x}",
                ap.startup_mp_state(),
                ap.startup_rip(),
                ap.startup_cs_selector(),
                ap.startup_cs_base()
            );
            println!("sipi-work AP ready rflags: {:#x}", ap.ready_rflags());
            println!("sipi-work AP completion rflags: {:#x}", ap.completion_rflags());
            println!(
                "sipi-work AP long mode: rsp={:#x} cs={:#x} L={} ss={:#x} gdt={:#x}/{:#x} idt={:#x}/{:#x} cr3={:#x}",
                ap.rsp(),
                ap.cs_selector(),
                ap.cs_long(),
                ap.ss_selector(),
                ap.gdt_base(),
                ap.gdt_limit(),
                ap.idt_base(),
                ap.idt_limit(),
                ap.cr3()
            );
            println!("sipi-work BSP report: {}", result.bsp_report());
            println!("sipi-work AP report: {}", result.ap_report());

            let valid = result.bsp_proof() == BSP_COMPOSED_PROOF
                && result.ap_proof() == AP_COMPOSED_PROOF
                && mailbox.payload() == WORK_PAYLOAD
                && mailbox.command() == 0
                && mailbox.result() == WORK_RESULT
                && mailbox.ack() == 0
                && result.initial_ap_mp_state() == 1
                && ap.startup_mp_state() == 0
                && ap.startup_rip() == 0
                && ap.startup_cs_selector() == 0x0800
                && ap.startup_cs_base() == 0x8000
                && result.bsp_report().exit() == VcpuExit::Hlt
                && result.bsp_report().rip() == BSP_TERMINAL_RIP
                && result.ap_report().exit() == VcpuExit::Hlt
                && result.ap_report().rip() == AP_TERMINAL_RIP;

            if valid {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: SIPI/IPI work-dispatch proof violated its fixed contract");
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
