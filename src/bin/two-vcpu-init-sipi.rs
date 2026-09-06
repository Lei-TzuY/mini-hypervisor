use mini_hypervisor::portio::two_vcpu_init_sipi_fixture::{
    run_two_vcpu_init_sipi, FIRST_PROOF, SECOND_PROOF, SIPI_VECTOR,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_init_sipi() {
        Ok(result) => {
            println!("INIT/SIPI vector: {SIPI_VECTOR:#x}");
            println!(
                "INIT/SIPI trampoline GPA: {:#x}",
                u64::from(SIPI_VECTOR) << 12
            );
            println!(
                "INIT/SIPI initial AP mp-state: {}",
                result.initial_mp_state()
            );
            println!(
                "INIT/SIPI startup AP mp-state: {}",
                result.startup_mp_state()
            );
            println!("INIT/SIPI startup AP RIP: {:#x}", result.startup_rip());
            println!(
                "INIT/SIPI startup AP CS selector: {:#x}",
                result.startup_cs_selector()
            );
            println!(
                "INIT/SIPI startup AP CS base: {:#x}",
                result.startup_cs_base()
            );
            println!("INIT/SIPI startup AP CR0: {:#x}", result.startup_cr0());
            println!("INIT/SIPI final AP mp-state: {}", result.final_mp_state());
            println!("INIT/SIPI shared marker: {:#x}", result.shared_marker());
            println!(
                "INIT/SIPI AP completion rflags: {:#x}",
                result.ap_completion_rflags()
            );
            println!("INIT/SIPI BSP proof: {:?}", result.first_proof());
            println!("INIT/SIPI AP proof: {:?}", result.second_proof());

            if result.first_proof() == FIRST_PROOF
                && result.second_proof() == SECOND_PROOF
                && result.shared_marker() == b'K'
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: INIT/SIPI executable proof did not match its fixed contract");
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
