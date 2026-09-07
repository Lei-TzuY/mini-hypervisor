use mini_hypervisor::address_space::run_address_space_switch_guest;
use mini_hypervisor::config::VmConfig;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_address_space_switch_guest(VmConfig::default()) {
        Ok(result) => {
            let cr3 = result.cr3_observations();
            println!("address-space roots: first=0x1000 second=0xb000");
            println!(
                "address-space CR3 observations: [{:#x}, {:#x}, {:#x}]",
                cr3[0], cr3[1], cr3[2]
            );
            println!("address-space final CR3: {:#x}", result.final_cr3());
            println!(
                "address-space data: first={} second={}",
                result.first_data(),
                result.second_data()
            );
            println!("address-space A code PTE: {:#x}", result.first_code_pte());
            println!("address-space B code PTE: {:#x}", result.second_code_pte());
            println!("address-space A data PTE: {:#x}", result.first_data_pte());
            println!("address-space B data PTE: {:#x}", result.second_data_pte());
            println!("address-space A stack PTE: {:#x}", result.first_stack_pte());
            println!("address-space B stack PTE: {:#x}", result.second_stack_pte());
            println!(
                "address-space A switch-handler PTE: {:#x}",
                result.first_kernel_pte()
            );
            println!(
                "address-space B switch-handler PTE: {:#x}",
                result.second_kernel_pte()
            );
            println!("address-space proof: {:?}", result.proof());
            println!("{}", result.report());
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
