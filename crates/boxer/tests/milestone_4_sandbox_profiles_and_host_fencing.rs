//! Milestone 4: Sandbox profile isolation, host mode fencing, and ephemeral scratch cleanup.

use std::path::Path;

use pulsar_boxer::{
    error::BoxerError,
    leases::CleanupDropGuard,
    sandbox::{
        HOST_UNCONFINED_ACK_VALUE, build_linux_landlock_spec, generate_macos_seatbelt_profile,
        policy::resolve_confined_workdir, validate_hardened_host_mode,
    },
};

#[test]
fn test_sandbox_profile_vfs_escape_rejection() {
    let tmp = tempfile::tempdir().expect("tempdir creation should succeed");
    let ws_root = tmp.path().canonicalize().expect("canonicalize ws root");
    let session_id = "session_iso_01";

    // Valid nested subdir inside workspace
    let valid_workdir = resolve_confined_workdir(&ws_root, session_id, Some("src/modules"));
    assert!(valid_workdir.is_ok());
    assert!(valid_workdir.unwrap().starts_with(&ws_root));

    // Absolute path traversal attempt
    let abs_attempt = resolve_confined_workdir(&ws_root, session_id, Some("/etc/shadow"));
    assert!(matches!(
        abs_attempt,
        Err(BoxerError::IsolationViolation(_))
    ));

    // Relative escape traversal attempt (../../)
    let escape_attempt = resolve_confined_workdir(&ws_root, session_id, Some("../../outside"));
    assert!(matches!(
        escape_attempt,
        Err(BoxerError::IsolationViolation(_))
    ));

    // Malicious session ID with traversal
    let bad_session = resolve_confined_workdir(&ws_root, "../bad_session", Some("code"));
    assert!(matches!(
        bad_session,
        Err(BoxerError::IsolationViolation(_))
    ));

    let slash_session = resolve_confined_workdir(&ws_root, "bad/session", Some("code"));
    assert!(matches!(
        slash_session,
        Err(BoxerError::IsolationViolation(_))
    ));
}

#[test]
fn test_sandbox_profile_isolation_modes() {
    let ws = Path::new("/tmp/pulsar-boxes/test-session");
    let skills = Path::new("/opt/skills");

    // Landlock Linux profile verification
    let landlock = build_linux_landlock_spec(ws, Some(skills), Some(8080));
    assert!(landlock.raw_tcp_banned);
    assert_eq!(landlock.allowed_loopback_port, Some(8080));

    // Workspace write is allowed
    assert!(
        landlock
            .verify_path_access(Path::new("/tmp/pulsar-boxes/test-session/output.txt"), true)
            .is_ok()
    );

    // Skills directory is read-only (write must fail)
    assert!(
        landlock
            .verify_path_access(Path::new("/opt/skills/git/SKILL.md"), false)
            .is_ok()
    );
    assert!(
        landlock
            .verify_path_access(Path::new("/opt/skills/git/SKILL.md"), true)
            .is_err()
    );

    // Host sensitive paths must be denied for both read and write
    assert!(
        landlock
            .verify_path_access(Path::new("/etc/passwd"), false)
            .is_err()
    );
    assert!(
        landlock
            .verify_path_access(Path::new("/root/.ssh/id_rsa"), false)
            .is_err()
    );

    // Seatbelt macOS profile verification
    let seatbelt = generate_macos_seatbelt_profile(ws, false);
    assert!(seatbelt.scheme_source.contains("(deny default)"));
    assert!(seatbelt.scheme_source.contains("(deny network*)"));
}

#[test]
fn test_host_mode_fencing_fails_closed() {
    // 1. Consent false: must fail closed even if token is provided
    let denied_no_consent = validate_hardened_host_mode(false, Some(HOST_UNCONFINED_ACK_VALUE));
    assert!(
        denied_no_consent.is_err(),
        "unconfined host mode without consent must fail closed"
    );

    // 2. Consent true without unconfined token: must remain sandboxed (not unconfined)
    let confined_default = validate_hardened_host_mode(true, None).unwrap();
    assert!(
        confined_default.sandboxed,
        "host mode without ACK token must remain sandboxed"
    );
    assert!(confined_default.warning_banner.is_none());

    // 3. Consent true with invalid token: remains sandboxed
    let confined_bad_token = validate_hardened_host_mode(true, Some("wrong-token")).unwrap();
    assert!(
        confined_bad_token.sandboxed,
        "host mode with mismatched ACK token must remain sandboxed"
    );
    assert!(confined_bad_token.warning_banner.is_none());

    // 4. Consent true and correct token: succeeds in unconfined mode with security warning
    let allowed_unconfined =
        validate_hardened_host_mode(true, Some(HOST_UNCONFINED_ACK_VALUE)).unwrap();
    assert!(allowed_unconfined.allowed);
    assert!(!allowed_unconfined.sandboxed);
    assert!(allowed_unconfined.warning_banner.is_some());
}

#[test]
fn test_scratch_cleanup_drop_guard() {
    let scratch_path =
        std::env::temp_dir().join(format!("pulsar-test-scratch-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&scratch_path).expect("create scratch dir");
    let test_file = scratch_path.join("artifact.log");
    std::fs::write(&test_file, b"ephemeral test output").expect("write test file");
    assert!(test_file.exists());

    // Enclose in scope with CleanupDropGuard
    {
        let to_clean = scratch_path.clone();
        let _guard = CleanupDropGuard::new(to_clean, |p| {
            let _ = std::fs::remove_dir_all(p);
        });
        assert!(test_file.exists());
    }

    // After guard drops, scratch dir must be completely erased
    assert!(
        !scratch_path.exists(),
        "scratch directory must be erased after drop guard runs"
    );
}
