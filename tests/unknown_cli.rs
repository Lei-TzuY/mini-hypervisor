use std::process::Command;

#[test]
fn unknown_cli_command_reports_usage_and_fails_without_touching_kvm() {
    let output = Command::new(env!("CARGO_BIN_EXE_mini-hypervisor"))
        .arg("definitely-not-a-command")
        .output()
        .expect("mini-hypervisor binary should launch");

    let stdout = String::from_utf8(output.stdout).expect("CLI stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("CLI stderr should be UTF-8");

    assert_eq!(output.status.code(), Some(2));
    assert!(stdout.is_empty());
    assert!(stderr.contains("usage: mini-hypervisor"));
    assert!(stderr.contains("unknown command: definitely-not-a-command"));
    assert!(!stderr.starts_with("error: "));
}
