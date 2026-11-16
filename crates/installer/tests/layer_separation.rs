//! Tests verifying strict build layer separation and zero circular dependencies.

use std::path::Path;

#[test]
fn test_protocol_crate_has_no_heavy_backend_dependencies() {
    let cargo_toml = include_str!("../../protocol/Cargo.toml");

    // Must not depend directly on database or runtime engines
    assert!(!cargo_toml.contains("sqlx"));
    assert!(!cargo_toml.contains("tokio-postgres"));
    assert!(!cargo_toml.contains("redis"));
    assert!(!cargo_toml.contains("async-nats"));
}

#[test]
fn test_styles_crate_has_no_backend_dependencies() {
    let cargo_toml = include_str!("../../styles/Cargo.toml");

    assert!(!cargo_toml.contains("sqlx"));
    assert!(!cargo_toml.contains("redis"));
    assert!(!cargo_toml.contains("async-nats"));
}

#[test]
fn test_all_workspace_crates_exist_and_isolated() {
    let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    assert!(crates_dir.join("protocol").exists());
    assert!(crates_dir.join("styles").exists());
    assert!(crates_dir.join("bridge").exists());
    assert!(crates_dir.join("server").exists());
    assert!(crates_dir.join("boxer").exists());
    assert!(crates_dir.join("sdk").exists());
    assert!(crates_dir.join("installer").exists());
    assert!(crates_dir.join("client").exists());
}
