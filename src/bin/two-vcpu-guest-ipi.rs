use mini_hypervisor::portio::two_vcpu_guest_ipi_fixture::{
    run_two_vcpu_guest_ipi, FIRST_PROOF, ICR_HIGH_VALUE, ICR_LOW_VALUE, LAPIC_GPA,
    LAPIC_VIRTUAL_PAGE, SECOND_PROOF, TARGET_APIC_ID, TARGET_VECTOR,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_two_vcpu_guest_ipi() {
        Ok(result) => {
            println!("guest IPI LAPIC alias: {LAPIC_VIRTUAL_PAGE:#x}");
            println!("guest IPI LAPIC GPA: {LAPIC_GPA:#x}");
            println!("guest IPI destination APIC ID: {TARGET_APIC_ID}");
            println!("guest IPI vector: {TARGET_VECTOR:#x}");
            println!("guest IPI ICR high: {ICR_HIGH_VALUE:#x}");
            println!("guest IPI ICR low: {ICR_LOW_VALUE:#x}");
            println!(
                "guest IPI second runnable mp-state: {}",
                result.second_mp_state()
            );
            println!("guest IPI first proof: {:?}", result.first_proof());
            println!("guest IPI second proof: {:?}", result.second_proof());
            println!(
                "guest IPI first barrier rflags: {:#x}",
                result.first_barrier_rflags()
            );
            println!(
                "guest IPI first send rflags: {:#x}",
                result.first_send_rflags()
            );
            println!(
                "guest IPI first completion rflags: {:#x}",
                result.first_completion_rflags()
            );
            println!(
                "guest IPI second ready rflags: {:#x}",
                result.second_ready_rflags()
            );
            println!(
                "guest IPI second completion rflags: {:#x}",
                result.second_completion_rflags()
            );

            if result.second_mp_state() == 0
                && result.first_proof() == FIRST_PROOF
                && result.second_proof() == SECOND_PROOF
            {
                ExitCode::SUCCESS
            } else {
                eprintln!("error: guest IPI execution proof violated its fixed contract");
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
