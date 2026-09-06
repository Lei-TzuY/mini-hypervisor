use mini_hypervisor::config::VmConfig;
use mini_hypervisor::syscall::usercopy::run_fault_safe_usercopy_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_fault_safe_usercopy_guest(VmConfig::default()) {
        Ok(result) => {
            println!("usercopy proof: {:?}", result.proof());
            println!(
                "usercopy results: good={:#x} bad_src={:#x} bad_dst={:#x} readback={:#x} source={:#x} destination={:#x}",
                result.good_return(),
                result.bad_source_return(),
                result.bad_destination_return(),
                result.user_readback(),
                result.source_value(),
                result.destination_value()
            );
            let read = result.read_fault();
            println!(
                "usercopy read fault: cr2={:#x} error={:#x} rip={:#x} cs={:#x} rflags={:#x} fixup={:#x}",
                read.cr2(), read.error_code(), read.rip(), read.cs(), read.rflags(), read.resolved_fixup()
            );
            let write = result.write_fault();
            println!(
                "usercopy write fault: cr2={:#x} error={:#x} rip={:#x} cs={:#x} rflags={:#x} fixup={:#x}",
                write.cr2(), write.error_code(), write.rip(), write.cs(), write.rflags(), write.resolved_fixup()
            );
            for (index, entry) in result.fixup_entries().iter().enumerate() {
                println!(
                    "usercopy fixup[{index}]: fault={:#x} fixup={:#x} observation={:#x}",
                    entry.fault_rip(),
                    entry.fixup_rip(),
                    entry.observation_addr()
                );
            }
            let frame = result.terminal_frame();
            println!(
                "usercopy terminal frame: rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
                frame.rip(),
                frame.cs(),
                frame.rflags(),
                frame.rsp(),
                frame.ss()
            );
            println!(
                "usercopy terminal: rsp={:#x} cs={:#x} rflags={:#x} cr2={:#x}",
                result.terminal_rsp(),
                result.terminal_cs(),
                result.terminal_rflags(),
                result.final_cr2()
            );
            println!(
                "usercopy MSRs: efer={:#x} star={:#x} lstar={:#x} sfmask={:#x}",
                result.efer(),
                result.star(),
                result.lstar(),
                result.sfmask()
            );
            println!("usercopy user page PTE: {:#x}", result.user_page_pte());
            println!(
                "usercopy fault handler PTE: {:#x}",
                result.fault_handler_pte()
            );
            println!(
                "usercopy fault metadata PTE: {:#x}",
                result.fault_metadata_pte()
            );
            println!("usercopy bad PD entry: {:#x}", result.bad_pd_entry());
            println!(
                "usercopy terminal report: rip={:#x} rflags={:#x}",
                result.report().rip(),
                result.report().rflags()
            );
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
