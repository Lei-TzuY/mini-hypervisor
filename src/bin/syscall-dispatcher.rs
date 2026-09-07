use mini_hypervisor::config::VmConfig;
use mini_hypervisor::syscall::dispatcher::run_syscall_dispatch_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_syscall_dispatch_guest(VmConfig::default()) {
        Ok(result) => {
            println!("dispatch proof: {:?}", result.proof());
            println!(
                "dispatch results: copy={:#x} readback={:#x} putc={:#x} bad_src={:#x} bad_dst={:#x} unknown={:#x} source={:#x} destination={:#x}",
                result.copy_return(),
                result.copy_readback(),
                result.putc_return(),
                result.bad_source_return(),
                result.bad_destination_return(),
                result.unknown_return(),
                result.source_value(),
                result.destination_value()
            );
            let read = result.read_fault();
            println!(
                "dispatch read fault: cr2={:#x} error={:#x} rip={:#x} cs={:#x} rflags={:#x} fixup={:#x}",
                read.cr2(),
                read.error_code(),
                read.rip(),
                read.cs(),
                read.rflags(),
                read.resolved_fixup()
            );
            let write = result.write_fault();
            println!(
                "dispatch write fault: cr2={:#x} error={:#x} rip={:#x} cs={:#x} rflags={:#x} fixup={:#x}",
                write.cr2(),
                write.error_code(),
                write.rip(),
                write.cs(),
                write.rflags(),
                write.resolved_fixup()
            );
            for (index, entry) in result.fixup_entries().iter().enumerate() {
                println!(
                    "dispatch fixup[{index}]: fault={:#x} fixup={:#x} observation={:#x}",
                    entry.fault_rip(),
                    entry.fixup_rip(),
                    entry.observation_addr()
                );
            }
            let frame = result.terminal_frame();
            println!(
                "dispatch terminal frame: rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
                frame.rip(),
                frame.cs(),
                frame.rflags(),
                frame.rsp(),
                frame.ss()
            );
            println!(
                "dispatch terminal: rsp={:#x} cs={:#x} rflags={:#x} cr2={:#x}",
                result.terminal_rsp(),
                result.terminal_cs(),
                result.terminal_rflags(),
                result.final_cr2()
            );
            println!(
                "dispatch MSRs: efer={:#x} star={:#x} lstar={:#x} sfmask={:#x}",
                result.efer(),
                result.star(),
                result.lstar(),
                result.sfmask()
            );
            println!("dispatch user page PTE: {:#x}", result.user_page_pte());
            println!("dispatch dispatcher PTE: {:#x}", result.dispatcher_pte());
            println!(
                "dispatch fault handler PTE: {:#x}",
                result.fault_handler_pte()
            );
            println!(
                "dispatch fault metadata PTE: {:#x}",
                result.fault_metadata_pte()
            );
            println!("dispatch bad PD entry: {:#x}", result.bad_pd_entry());
            println!(
                "dispatch terminal report: rip={:#x} rflags={:#x}",
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
