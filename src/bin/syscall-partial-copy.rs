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
                "partial-copy read fault: cr2={:#x} error={:#x} rip={:#x} rflags={:#x} fixup={:#x}",
                read.cr2(),
                read.error_code(),
                read.rip(),
                read.rflags(),
                read.resolved_fixup()
            );
            let write = result.write_fault();
            println!(
                "partial-copy write fault: cr2={:#x} error={:#x} rip={:#x} rflags={:#x} fixup={:#x}",
                write.cr2(), write.error_code(), write.rip(), write.rflags(), write.resolved_fixup()
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
