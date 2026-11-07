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

#[test]
fn self_update_preflights_current_binary_and_preserves_catalog() {
    use nexus_launcher::catalog::current_platform;
    use nexus_launcher::self_update::{apply_verified_archive, official_binary_name, ArchiveApply};

    let dir = std::env::temp_dir().join(format!("nexus-launcher-preflight-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("catalog")).unwrap();
    let catalog = dir.join("catalog/apps.toml");
    std::fs::write(&catalog, "owner's custom catalog").unwrap();
    let target = dir.join(official_binary_name(current_platform()));
    std::fs::write(&target, "previous executable").unwrap();
    let archive = dir.join("launcher.tar.gz");
    let file = std::fs::File::create(&archive).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut tar = tar::Builder::new(encoder);
    tar.append_path_with_name(
        env!("CARGO_BIN_EXE_nexus-launcher"),
        format!(
            "nexus-launcher-package/{}",
            official_binary_name(current_platform())
        ),
    )
    .unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    let sha = nexus_launcher::checksum::sha256_file(&archive).unwrap();

    apply_verified_archive(&ArchiveApply {
        archive: &archive,
        expected_sha256: &sha,
        target_exe: &target,
        staging_dir: &dir.join("staging"),
        state_path: &dir.join("applied.json"),
        tag: concat!("v", env!("CARGO_PKG_VERSION")),
        asset_name: "launcher.tar.gz",
        platform: current_platform(),
        preflight: true,
    })
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(catalog).unwrap(),
        "owner's custom catalog"
    );
    assert_eq!(
        std::fs::metadata(&target).unwrap().len(),
        std::fs::metadata(env!("CARGO_BIN_EXE_nexus-launcher"))
            .unwrap()
            .len()
    );
    assert!(Command::new(&target)
        .arg("--help")
        .status()
        .unwrap()
        .success());
    std::fs::remove_dir_all(dir).unwrap();
}
