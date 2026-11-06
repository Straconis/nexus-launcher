//! The self-updater runs `nexus-launcher --check` on a downloaded build
//! before it replaces the running file. This exercises that command.

use std::process::Command;

#[test]
fn launcher_check_exits_cleanly() {
    let bin = env!("CARGO_BIN_EXE_nexus-launcher");
    let dir = std::env::temp_dir().join(format!("nexus-launcher-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let catalog = dir.join("apps.toml");
    std::fs::write(
        &catalog,
        "[[app]]\nid = \"stat-tracker\"\nname = \"Stat Tracker\"\n",
    )
    .unwrap();
    let output = Command::new(bin)
        .arg("--check")
        .arg(&catalog)
        .output()
        .expect("spawn launcher");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("catalog ok"));
    assert!(stdout.contains("stat-tracker"));
    let _ = std::fs::remove_dir_all(&dir);
}
