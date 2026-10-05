//! The S6b probe's scorer (task 13.1) is tested without E3D or a license: page parsing,
//! per-run markers, the validity/observation split and the branch table.
#[test]
fn s6b_offline_probe_contracts() {
    if !cfg!(windows) {
        return; // PowerShell / E3D probes are Windows-only.
    }
    let output = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-File", "tools/e3d/s6b_selfcheck.ps1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("PowerShell 7 must be available for Windows probe checks");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("PASS S6B offline"));
}
