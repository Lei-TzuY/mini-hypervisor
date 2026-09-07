use mini_hypervisor::config::VmConfig;
use mini_hypervisor::syscall::partial_dispatch::run_partial_dispatch_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_partial_dispatch_guest(VmConfig::default()) {
        Ok(result) => {
            println!("partial-copy proof: {:?}", result.proof());
            println!("partial-copy returns: {:?}", result.returns());
            println!(
                "partial-copy good destination: {:?}",
                result.good_destination()
            );
            println!(
                "partial-copy source-fault destination: {:?}",
                result.source_fault_destination()
            );
            println!(
                "partial-copy destination-fault destination: {:?}",
                result.destination_fault_destination()
            );
            println!(
                "partial-copy short destination: {:?}",
                result.short_destination()
            );
            println!(
                "partial-copy byte destination: {:#x}",
                result.byte_destination()
            );
            let read = result.read_fault();
            println!(
                "partial-copy read fault: cr2={:#x} error={:#x} rip={:#x} cs={:#x} rflags={:#x} fixup={:#x}",
                read.cr2(),
                read.error_code(),
                read.rip(),
                read.cs(),
                read.rflags(),
                read.resolved_fixup()
            );
            let write = result.write_fault();
            println!(
                "partial-copy write fault: cr2={:#x} error={:#x} rip={:#x} cs={:#x} rflags={:#x} fixup={:#x}",
                write.cr2(),
                write.error_code(),
                write.rip(),
                write.cs(),
                write.rflags(),
                write.resolved_fixup()
            );
            for (index, entry) in result.fixup_entries().iter().enumerate() {
                println!(
                    "partial-copy fixup[{index}]: fault={:#x} fixup={:#x} observation={:#x}",
                    entry.fault_rip(),
                    entry.fixup_rip(),
                    entry.observation_addr()
                );
            }
            for (address, pte) in result.user_page_ptes() {
                println!("partial-copy PTE {address:#x}: {pte:#x}");
            }
            println!("partial-copy service PTE: {:#x}", result.service_pte());
            println!(
                "partial-copy fault handler PTE: {:#x}",
                result.fault_handler_pte()
            );
            println!(
                "partial-copy fault metadata PTE: {:#x}",
                result.fault_metadata_pte()
            );
            let frame = result.terminal_frame();
            println!(
                "partial-copy terminal frame: rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
                frame.rip(),
                frame.cs(),
                frame.rflags(),
                frame.rsp(),
                frame.ss()
            );
            println!(
                "partial-copy terminal: rsp={:#x} cs={:#x} rflags={:#x} cr2={:#x}",
                result.terminal_rsp(),
                result.terminal_cs(),
                result.terminal_rflags(),
                result.final_cr2()
            );
            let [efer, star, lstar, sfmask] = result.msrs();
            println!(
                "partial-copy MSRs: efer={efer:#x} star={star:#x} lstar={lstar:#x} sfmask={sfmask:#x}"
            );
            println!("partial-copy terminal report: {}", result.report());
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
