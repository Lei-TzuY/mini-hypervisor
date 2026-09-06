use mini_hypervisor::long_mode::{
    LONG_MODE_CR0_REQUIRED_BITS, LONG_MODE_CR4_REQUIRED_BITS, LONG_MODE_EFER_REQUIRED_BITS,
    LONG_MODE_PML4_ADDR,
};
use mini_hypervisor::portio::two_vcpu_init_sipi_fixture::{
    run_two_vcpu_ap_long_mode, AP_LONG_MODE_CODE_SELECTOR, AP_LONG_MODE_GDT,
    AP_LONG_MODE_GDT_LIMIT, AP_LONG_MODE_PROOF, AP_LONG_MODE_STACK, FIRST_PROOF, SIPI_VECTOR,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_ap_long_mode() {
        Ok(result) => {
            let state = result.long_mode_state();
            println!("AP long-mode SIPI vector: {SIPI_VECTOR:#x}");
            println!("AP long-mode startup CS selector: {:#x}", result.startup_cs_selector());
            println!("AP long-mode startup CS base: {:#x}", result.startup_cs_base());
            println!("AP long-mode startup CR0: {:#x}", result.startup_cr0());
            println!("AP long-mode stack: {:#x}", state.rsp());
            println!("AP long-mode CS selector: {:#x}", state.cs_selector());
            println!("AP long-mode CS.L: {}", state.cs_long());
            println!("AP long-mode SS selector: {:#x}", state.ss_selector());
            println!("AP long-mode GDT base: {:#x}", state.gdt_base());
            println!("AP long-mode GDT limit: {:#x}", state.gdt_limit());
            println!("AP long-mode CR0: {:#x}", state.cr0());
            println!("AP long-mode CR3: {:#x}", state.cr3());
            println!("AP long-mode CR4: {:#x}", state.cr4());
            println!("AP long-mode EFER: {:#x}", state.efer());
            println!("AP long-mode final mp-state: {}", result.final_mp_state());
            println!("AP long-mode marker: {:#x}", result.shared_marker());
            println!("AP long-mode completion rflags: {:#x}", result.ap_completion_rflags());
            println!("AP long-mode BSP proof: {:?}", result.first_proof());
            println!("AP long-mode AP proof: {:?}", result.second_proof());

            let valid = result.first_proof() == FIRST_PROOF
                && result.second_proof() == AP_LONG_MODE_PROOF
                && result.shared_marker() == b'K'
                && state.rsp() == AP_LONG_MODE_STACK
                && state.cs_selector() == AP_LONG_MODE_CODE_SELECTOR
                && state.cs_long() == 1
                && state.gdt_base() == AP_LONG_MODE_GDT.get()
                && state.gdt_limit() == AP_LONG_MODE_GDT_LIMIT
                && state.cr0() & LONG_MODE_CR0_REQUIRED_BITS == LONG_MODE_CR0_REQUIRED_BITS
                && state.cr3() == LONG_MODE_PML4_ADDR.get()
                && state.cr4() & LONG_MODE_CR4_REQUIRED_BITS == LONG_MODE_CR4_REQUIRED_BITS
                && state.efer() & LONG_MODE_EFER_REQUIRED_BITS == LONG_MODE_EFER_REQUIRED_BITS;
            if valid {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: AP guest-driven long-mode executable proof did not match its fixed contract");
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
