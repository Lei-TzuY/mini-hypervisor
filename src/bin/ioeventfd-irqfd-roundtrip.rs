use mini_hypervisor::config::VmConfig;
use mini_hypervisor::kvm::KvmBackend;
use std::process::ExitCode;

fn main() -> ExitCode {
    match KvmBackend::run_ioeventfd_irqfd_roundtrip_guest(VmConfig::default()) {
        Ok(result) => {
            println!("ioeventfd capability: KVM_CAP_IOEVENTFD");
            println!("irqfd capability: KVM_CAP_IRQFD");
            println!("round-trip doorbell GPA: {:#x}", result.doorbell_gpa());
            println!("round-trip doorbell value: {}", result.doorbell_value());
            println!("round-trip doorbell events: {}", result.doorbell_events());
            println!("round-trip GSI: {}", result.gsi());
            println!("round-trip vector: {:#x}", result.vector());
            println!("round-trip LAPIC SPIV: {:#x}", result.lapic_spiv());
            println!("round-trip LAPIC LINT0: {:#x}", result.lapic_lint0());
            println!("round-trip armed rflags: {:#x}", result.armed_rflags());
            println!(
                "round-trip completion rflags: {:#x}",
                result.completion_rflags()
            );
            println!("round-trip proof: {:?}", result.proof());
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
