use mini_hypervisor::config::VmConfig;
use mini_hypervisor::mmio::dual_source_interrupt::run_dual_source_mmio_interrupt_guest;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_dual_source_mmio_interrupt_guest(VmConfig::default()) {
        Ok(result) => {
            let routes = result.routes();
            println!(
                "dual-source first route: device={:#x} gsi={} vector={:#x}",
                routes[0].device_address(),
                routes[0].gsi(),
                routes[0].vector()
            );
            println!(
                "dual-source second route: device={:#x} gsi={} vector={:#x}",
                routes[1].device_address(),
                routes[1].gsi(),
                routes[1].vector()
            );
            println!("dual-source assert events: {}", result.assert_event_count());
            println!(
                "dual-source deassert events: {}",
                result.deassert_event_count()
            );
            println!("dual-source MMIO exits: {}", result.mmio_exits().len());
            println!("dual-source first writes: {:?}", result.first_writes());
            println!("dual-source second writes: {:?}", result.second_writes());
            println!("dual-source LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("dual-source LAPIC LINT0: {:#x}", result.lapic_lint0());
            let armed = result.armed_rflags();
            println!("dual-source first armed rflags: {:#x}", armed[0]);
            println!("dual-source second armed rflags: {:#x}", armed[1]);
            println!(
                "dual-source completion rflags: {:#x}",
                result.completion_rflags()
            );
            println!("dual-source proof: {:?}", result.proof());
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
