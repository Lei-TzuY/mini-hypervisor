use std::process::Command;

#[test]
fn run_cpuid_cli_is_recognized_and_reports_existing_guest_proof_when_kvm_is_available() {
    let output = Command::new(env!("CARGO_BIN_EXE_mini-hypervisor"))
        .arg("run-cpuid")
        .output()
        .expect("mini-hypervisor binary should launch");
    let stdout = String::from_utf8(output.stdout).expect("CLI stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("CLI stderr should be UTF-8");

    assert!(!stderr.contains("unknown command: run-cpuid"));
    assert!(!stderr.contains("usage: mini-hypervisor"));

    if output.status.success() {
        assert!(stdout.contains("cpuid(1).ecx: 0x"));
        assert!(stdout.contains("cpuid(0x40000001).eax: 0x"));
        assert!(stdout.contains("masked LAPIC-dependent features clear: true"));
        assert!(stdout.contains("vCPU 0 exit Hlt:"));
    } else {
        assert!(stderr.starts_with("error: "));
    }
}
