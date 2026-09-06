use mini_hypervisor::portio::two_vcpu_ap_local_timer_fixture::{
    run_two_vcpu_ap_local_timer, AP_LOCAL_TIMER_DIVIDE_CONFIGURATION, AP_LOCAL_TIMER_INITIAL_COUNT,
    AP_LOCAL_TIMER_VECTOR,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_ap_local_timer() {
        Ok(result) => {
            let state = result.ap_state();
            println!("AP local timer vector: {AP_LOCAL_TIMER_VECTOR:#x}");
            println!(
                "AP local timer divide configuration: {AP_LOCAL_TIMER_DIVIDE_CONFIGURATION:#x}"
            );
            println!("AP local timer initial count: {AP_LOCAL_TIMER_INITIAL_COUNT:#x}");
            println!("AP local timer watchdog fired: {}", result.watchdog_fired());
            println!(
                "AP local timer initial MP state: {}",
                result.initial_ap_mp_state()
            );
            println!(
                "AP local timer startup: mp={} rip={:#x} cs={:#x} base={:#x}",
                state.startup_mp_state(),
                state.startup_rip(),
                state.startup_cs_selector(),
                state.startup_cs_base()
            );
            println!(
                "AP local timer IDT: base={:#x} limit={:#x}",
                state.idt_base(),
                state.idt_limit()
            );
            println!("AP local timer ready rflags: {:#x}", state.ready_rflags());
            println!("AP local timer armed rflags: {:#x}", state.armed_rflags());
            println!(
                "AP local timer completion rflags: {:#x}",
                state.completion_rflags()
            );
            println!("AP local timer shared marker: {}", result.shared_marker());
            println!("AP local timer BSP proof: {:?}", result.bsp_proof());
            println!("AP local timer AP proof: {:?}", result.ap_proof());
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
