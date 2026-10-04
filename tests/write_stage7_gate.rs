//! The deterministic PowerShell gate scorer is tested without E3D or a license.
#[test]
fn q13_offline_gate_contracts() {
    if !cfg!(windows) {
        return; // PowerShell / E3D gates are Windows-only.
    }
    let output = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-File", "tools/e3d/q13_selfcheck.ps1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("PowerShell 7 must be available for Windows gate checks");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("PASS Q13 offline"));
}
