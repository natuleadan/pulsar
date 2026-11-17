//! Integration tests for sovereign multiplatform packaging scripts and zero-sudo constraints.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[test]
fn test_packaging_scripts_exist_and_executable() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let macos_script = repo_root.join("helpers/deploy/package-macos.sh");
    let linux_script = repo_root.join("helpers/deploy/package-linux.sh");

    assert!(
        macos_script.exists(),
        "package-macos.sh must exist in helpers/deploy"
    );
    assert!(
        linux_script.exists(),
        "package-linux.sh must exist in helpers/deploy"
    );

    let macos_meta = fs::metadata(&macos_script).unwrap();
    assert!(
        macos_meta.permissions().mode() & 0o111 != 0,
        "package-macos.sh must be executable"
    );

    let linux_meta = fs::metadata(&linux_script).unwrap();
    assert!(
        linux_meta.permissions().mode() & 0o111 != 0,
        "package-linux.sh must be executable"
    );
}

#[test]
fn test_packaging_scripts_comply_with_rule_g6_zero_sudo() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let macos_script = repo_root.join("helpers/deploy/package-macos.sh");
    let linux_script = repo_root.join("helpers/deploy/package-linux.sh");

    for script in &[&macos_script, &linux_script] {
        let content = fs::read_to_string(script).unwrap();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }
            assert!(
                !trimmed.starts_with("sudo ") && !trimmed.contains(" sudo "),
                "{:?} must not execute sudo commands: {line}",
                script
            );
            assert!(
                !trimmed.contains("brew install"),
                "{:?} must not invoke brew install: {line}",
                script
            );
        }
        assert!(
            content.lines().count() <= 500,
            "{:?} must be <= 500 lines (Rule G-20)",
            script
        );
    }
}
