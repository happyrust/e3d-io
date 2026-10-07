//! Unified regression parsing is checked without a license or E3D process.
#[test]
fn unified_regression_offline_contracts() {
    if !cfg!(windows) {
        return;
    }
    let output = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-File", "tools/e3d/regression_selfcheck.ps1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("PowerShell 7 must be available for Windows gate checks");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("PASS REGRESSION offline"));
}
