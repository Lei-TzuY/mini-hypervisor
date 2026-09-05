use mini_hypervisor::portio::two_vcpu_targeted_msi_fixture::{
    run_two_vcpu_targeted_msi_guest, FIRST_PROOF, SECOND_PROOF, TARGET_MSI_ADDRESS, TARGET_MSI_DATA,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_targeted_msi_guest() {
        Ok(result) => {
            println!("targeted MSI address: {:#x}", result.msi_address());
            println!("targeted MSI data: {:#x}", result.msi_data());
            println!("targeted MSI deliveries: {}", result.msi_delivery_count());
            println!(
                "targeted MSI second runnable mp-state: {}",
                result.second_mp_state()
            );
            println!("targeted MSI first proof: {:?}", result.first_proof());
            println!("targeted MSI second proof: {:?}", result.second_proof());
            println!(
                "targeted MSI first barrier rflags: {:#x}",
                result.first_barrier_rflags()
            );
            println!(
                "targeted MSI second ready rflags: {:#x}",
                result.second_ready_rflags()
            );
            println!(
                "targeted MSI second completion rflags: {:#x}",
                result.second_completion_rflags()
            );

            if result.msi_address() == TARGET_MSI_ADDRESS
                && result.msi_data() == TARGET_MSI_DATA
                && result.msi_delivery_count() == 1
                && result.second_mp_state() == 0
                && result.first_proof() == FIRST_PROOF
                && result.second_proof() == SECOND_PROOF
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: targeted MSI execution proof violated its fixed contract");
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
