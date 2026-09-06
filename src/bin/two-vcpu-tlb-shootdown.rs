use mini_hypervisor::portio::two_vcpu_tlb_shootdown_fixture::{
    run_two_vcpu_tlb_shootdown, TLB_SHOOTDOWN_VECTOR, TLB_TARGET_PTE,
    TLB_TARGET_VIRTUAL_PAGE,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_tlb_shootdown() {
        Ok(result) => {
            let state = result.state();
            println!("TLB shootdown vector: {TLB_SHOOTDOWN_VECTOR:#x}");
            println!("TLB shootdown target VA: {TLB_TARGET_VIRTUAL_PAGE:#x}");
            println!("TLB shootdown target PTE: {:#x}", TLB_TARGET_PTE.get());
            println!("TLB shootdown initial AP MP state: {}", state.initial_ap_mp_state());
            println!(
                "TLB shootdown startup: rip={:#x} cs={:#x} base={:#x}",
                state.startup_rip(),
                state.startup_cs_selector(),
                state.startup_cs_base()
            );
            println!(
                "TLB shootdown AP IDT: base={:#x} limit={:#x}",
                state.idt_base(),
                state.idt_limit()
            );
            println!("TLB shootdown ready rflags: {:#x}", state.ready_rflags());
            println!(
                "TLB shootdown completion rflags: {:#x}",
                state.completion_rflags()
            );
            println!("TLB shootdown final PTE: {:#x}", result.final_pte());
            println!("TLB shootdown final ack: {}", result.final_ack());
            println!("TLB shootdown page A: {}", result.page_a());
            println!("TLB shootdown page B: {}", result.page_b());
            println!("TLB shootdown BSP proof: {:?}", result.bsp_proof());
            println!("TLB shootdown AP proof: {:?}", result.ap_proof());
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
