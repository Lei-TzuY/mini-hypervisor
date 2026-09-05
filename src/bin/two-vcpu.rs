use mini_hypervisor::portio::two_vcpu_fixture::{
    run_two_vcpu_guest, FIRST_PROOF, FIRST_TERMINAL_RIP, FIRST_VCPU_ID, SECOND_PROOF,
    SECOND_TERMINAL_RIP, SECOND_VCPU_ID, SHARED_MARKER_VALUE,
};
use mini_hypervisor::vcpu::VcpuExit;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_guest() {
        Ok(result) => {
            println!("two-vcpu first proof: {:?}", result.first_proof());
            println!("two-vcpu second proof: {:?}", result.second_proof());
            println!("two-vcpu shared marker: {}", result.shared_marker());
            println!("two-vcpu first report: {}", result.first_report());
            println!("two-vcpu second report: {}", result.second_report());

            let first = result.first_report();
            let second = result.second_report();
            let valid = result.first_proof() == FIRST_PROOF
                && result.second_proof() == SECOND_PROOF
                && result.shared_marker() == SHARED_MARKER_VALUE
                && first.vcpu_id() == FIRST_VCPU_ID
                && second.vcpu_id() == SECOND_VCPU_ID
                && first.exit() == VcpuExit::Hlt
                && second.exit() == VcpuExit::Hlt
                && first.rip() == FIRST_TERMINAL_RIP
                && second.rip() == SECOND_TERMINAL_RIP
                && first.rflags() & 0x2 == 0x2
                && second.rflags() & 0x2 == 0x2;
            if valid {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: two-vCPU execution proof violated its fixed contract");
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
