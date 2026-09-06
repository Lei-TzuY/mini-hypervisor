use mini_hypervisor::interrupt::{LONG_MODE_INTERRUPT_IDT_ADDR, X86_RFLAGS_INTERRUPT_ENABLE};
use mini_hypervisor::long_mode::{
    LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS,
    LONG_MODE_PML4_ADDR,
};
use mini_hypervisor::portio::two_vcpu_init_sipi_fixture::{
    run_two_vcpu_ap_long_mode_ipi, AP_LONG_MODE_CODE_SELECTOR, AP_LONG_MODE_GDT,
    AP_LONG_MODE_GDT_LIMIT, AP_LONG_MODE_IPI_BSP_PROOF, AP_LONG_MODE_IPI_IDT_LIMIT,
    AP_LONG_MODE_IPI_PROOF, AP_LONG_MODE_IPI_VECTOR, AP_LONG_MODE_STACK, SIPI_VECTOR,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_ap_long_mode_ipi() {
        Ok(result) => {
            let long_mode = result.long_mode_state();
            let interrupt = result.interrupt_state();

            println!("AP IPI SIPI vector: {SIPI_VECTOR:#x}");
            println!("AP IPI vector: {AP_LONG_MODE_IPI_VECTOR:#x}");
            println!(
                "AP IPI startup CS selector: {:#x}",
                result.startup_cs_selector()
            );
            println!("AP IPI startup CS base: {:#x}", result.startup_cs_base());
            println!("AP IPI startup CR0: {:#x}", result.startup_cr0());
            println!("AP IPI stack: {:#x}", long_mode.rsp());
            println!("AP IPI CS selector: {:#x}", long_mode.cs_selector());
            println!("AP IPI CS.L: {}", long_mode.cs_long());
            println!("AP IPI SS selector: {:#x}", long_mode.ss_selector());
            println!("AP IPI GDT base: {:#x}", long_mode.gdt_base());
            println!("AP IPI GDT limit: {:#x}", long_mode.gdt_limit());
            println!("AP IPI CR0: {:#x}", long_mode.cr0());
            println!("AP IPI CR3: {:#x}", long_mode.cr3());
            println!("AP IPI CR4: {:#x}", long_mode.cr4());
            println!("AP IPI EFER: {:#x}", long_mode.efer());
            println!("AP IPI IDT base: {:#x}", interrupt.idt_base());
            println!("AP IPI IDT limit: {:#x}", interrupt.idt_limit());
            println!("AP IPI ready rflags: {:#x}", interrupt.ready_rflags());
            println!(
                "AP IPI completion rflags: {:#x}",
                result.ap_completion_rflags()
            );
            println!("AP IPI final mp-state: {}", result.final_mp_state());
            println!("AP IPI marker: {:#x}", result.shared_marker());
            println!("AP IPI BSP proof: {:?}", result.first_proof());
            println!("AP IPI AP proof: {:?}", result.second_proof());

            let valid = result.first_proof() == AP_LONG_MODE_IPI_BSP_PROOF
                && result.second_proof() == AP_LONG_MODE_IPI_PROOF
                && result.shared_marker() == b'K'
                && long_mode.rsp() == AP_LONG_MODE_STACK
                && long_mode.cs_selector() == AP_LONG_MODE_CODE_SELECTOR
                && long_mode.cs_long() == 1
                && long_mode.gdt_base() == AP_LONG_MODE_GDT.get()
                && long_mode.gdt_limit() == AP_LONG_MODE_GDT_LIMIT
                && long_mode.cr0() & LONG_MODE_CR0_REQUIRED_BITS == LONG_MODE_CR0_REQUIRED_BITS
                && long_mode.cr3() == LONG_MODE_PML4_ADDR.get()
                && long_mode.cr4() & LONG_MODE_CR4_REQUIRED_BITS == LONG_MODE_CR4_REQUIRED_BITS
                && long_mode.efer() & LONG_MODE_EFER_REQUIRED_BITS == LONG_MODE_EFER_REQUIRED_BITS
                && interrupt.idt_base() == LONG_MODE_INTERRUPT_IDT_ADDR.get()
                && interrupt.idt_limit() == AP_LONG_MODE_IPI_IDT_LIMIT
                && interrupt.ready_rflags() & 0x2 == 0x2
                && interrupt.ready_rflags() & X86_RFLAGS_INTERRUPT_ENABLE == 0
                && result.ap_completion_rflags() & 0x2 == 0x2
                && result.ap_completion_rflags() & X86_RFLAGS_INTERRUPT_ENABLE
                    == X86_RFLAGS_INTERRUPT_ENABLE;

            if valid {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: AP guest-originated long-mode IPI proof did not match its fixed contract");
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
