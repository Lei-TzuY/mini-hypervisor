use mini_hypervisor::config::VmConfig;
use mini_hypervisor::syscall::cross_page::run_cross_page_usercopy_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_cross_page_usercopy_guest(VmConfig::default()) {
        Ok(result) => {
            println!("cross-page proof: {:?}", result.proof());
            println!("cross-page returns: {:?}", result.returns());
            println!(
                "cross-page good: source={:?} destination={:?}",
                result.good_source(),
                result.good_destination()
            );
            println!(
                "cross-page source-fault: source={:?} destination={:?}",
                result.source_fault_source(),
                result.source_fault_destination()
            );
            println!(
                "cross-page destination-fault: source={:?} destination={:?}",
                result.destination_fault_source(),
                result.destination_fault_destination()
            );
            let read = result.read_fault();
            println!(
                "cross-page read fault: cr2={:#x} error={:#x} rip={:#x} cs={:#x} rflags={:#x} fixup={:#x}",
                read.cr2(),
                read.error_code(),
                read.rip(),
                read.cs(),
                read.rflags(),
                read.resolved_fixup()
            );
            let write = result.write_fault();
            println!(
                "cross-page write fault: cr2={:#x} error={:#x} rip={:#x} cs={:#x} rflags={:#x} fixup={:#x}",
                write.cr2(),
                write.error_code(),
                write.rip(),
                write.cs(),
                write.rflags(),
                write.resolved_fixup()
            );
            for (index, entry) in result.fixup_entries().iter().enumerate() {
                println!(
                    "cross-page fixup[{index}]: fault={:#x} fixup={:#x} observation={:#x}",
                    entry.fault_rip(),
                    entry.fixup_rip(),
                    entry.observation_addr()
                );
            }
            for (address, pte) in result.user_page_ptes() {
                println!("cross-page user PTE {address:#x}: {pte:#x}");
            }
            println!("cross-page service PTE: {:#x}", result.service_pte());
            println!(
                "cross-page fault handler PTE: {:#x}",
                result.fault_handler_pte()
            );
            println!(
                "cross-page fault metadata PTE: {:#x}",
                result.fault_metadata_pte()
            );
            let frame = result.terminal_frame();
            println!(
                "cross-page terminal frame: rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
                frame.rip(),
                frame.cs(),
                frame.rflags(),
                frame.rsp(),
                frame.ss()
            );
            println!(
                "cross-page terminal: rsp={:#x} cs={:#x} rflags={:#x} cr2={:#x}",
                result.terminal_rsp(),
                result.terminal_cs(),
                result.terminal_rflags(),
                result.final_cr2()
            );
            println!("cross-page MSRs: {:?}", result.msrs());
            println!(
                "cross-page terminal report: rip={:#x} rflags={:#x}",
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
