use std::process::Command;

#[test]
fn cli_without_command_prints_usage_and_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_e3d-descriptor"))
        .output()
        .expect("run descriptor CLI");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("e3d-descriptor describe"));
}
