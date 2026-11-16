use std::path::Path;

use crate::{error::BoxerError, sandbox::policy::binary_exists};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatbeltProfile {
    pub workspace_dir: String,
    pub allow_loopback_network: bool,
    pub scheme_source: String,
}

#[must_use]
pub fn generate_macos_seatbelt_profile(
    workspace_dir: &Path,
    allow_loopback_network: bool,
) -> SeatbeltProfile {
    let ws = workspace_dir.to_string_lossy().into_owned();
    let net_rule = if allow_loopback_network {
        "(allow network-outbound (remote ip \"localhost:*\"))"
    } else {
        "(deny network*)"
    };
    let scheme_source = format!(
        "(version 1)\n\
         (deny default)\n\
         (allow process-exec*)\n\
         (allow process-fork)\n\
         (allow signal (target self))\n\
         (allow sysctl-read)\n\
         (allow file-read* (subpath \"/usr\") (subpath \"/bin\") (subpath \"/sbin\") (subpath \"/lib\") (subpath \"/System\") (subpath \"/Library/Preferences\") (subpath \"/dev\") (subpath \"/private/tmp\") (subpath \"{ws}\"))\n\
         (allow file-write* (subpath \"/private/tmp\") (subpath \"/dev/null\") (subpath \"{ws}\"))\n\
         (deny file-read* (subpath \"/Users\"))\n\
         (allow file-read* (subpath \"{ws}\"))\n\
         {net_rule}\n"
    );
    SeatbeltProfile {
        workspace_dir: ws,
        allow_loopback_network,
        scheme_source,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandlockRulesetSpec {
    pub ro_paths: Vec<String>,
    pub rw_paths: Vec<String>,
    pub raw_tcp_banned: bool,
    pub allowed_loopback_port: Option<u16>,
}

#[must_use]
pub fn build_linux_landlock_spec(
    workspace_dir: &Path,
    skills_mount_dir: Option<&Path>,
    proxy_loopback_port: Option<u16>,
) -> LandlockRulesetSpec {
    let mut ro_paths = vec![
        "/usr".to_owned(),
        "/bin".to_owned(),
        "/sbin".to_owned(),
        "/lib".to_owned(),
        "/lib64".to_owned(),
        "/etc/alternatives".to_owned(),
    ];
    if let Some(skills) = skills_mount_dir {
        ro_paths.push(skills.to_string_lossy().into_owned());
    }
    let rw_paths = vec![
        workspace_dir.to_string_lossy().into_owned(),
        "/tmp".to_owned(),
    ];
    LandlockRulesetSpec {
        ro_paths,
        rw_paths,
        raw_tcp_banned: true,
        allowed_loopback_port: proxy_loopback_port,
    }
}

impl LandlockRulesetSpec {
    pub fn verify_path_access(&self, target_path: &Path, write: bool) -> Result<(), BoxerError> {
        let target_str = target_path.to_string_lossy();
        if target_str.contains("..")
            || target_str.contains("/.ssh")
            || target_str.contains("/Library")
            || target_str.contains("/etc/shadow")
        {
            return Err(BoxerError::IsolationViolation(format!(
                "PermissionDenied: Landlock/Seatbelt blocked access to `{target_str}`"
            )));
        }
        let in_rw = self
            .rw_paths
            .iter()
            .any(|rw| target_str == *rw || target_str.starts_with(&format!("{rw}/")));
        if write {
            if in_rw {
                return Ok(());
            }
            return Err(BoxerError::IsolationViolation(format!(
                "PermissionDenied: write access outside confined workspace `{target_str}`"
            )));
        }
        let in_ro = self
            .ro_paths
            .iter()
            .any(|ro| target_str == *ro || target_str.starts_with(&format!("{ro}/")));
        if in_rw || in_ro {
            Ok(())
        } else {
            Err(BoxerError::IsolationViolation(format!(
                "PermissionDenied: read access outside confined roots `{target_str}`"
            )))
        }
    }
}

pub const HOST_UNCONFINED_ACK_ENV: &str = "NLA_SANDBOX_MODE";
pub const HOST_UNCONFINED_ACK_VALUE: &str = "host_unconfined_ack";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostModeGuardVerdict {
    pub allowed: bool,
    pub sandboxed: bool,
    pub warning_banner: Option<String>,
}

pub fn validate_hardened_host_mode(
    allow_host_native: bool,
    explicit_env_val: Option<&str>,
) -> Result<HostModeGuardVerdict, BoxerError> {
    if !allow_host_native {
        return Err(BoxerError::IsolationViolation(
            "fail-closed: host runtime is disabled without explicit allow_host_native".into(),
        ));
    }
    let acked = explicit_env_val
        .map(str::trim)
        .is_some_and(|v| v == HOST_UNCONFINED_ACK_VALUE);
    if !acked && !has_os_native_sandbox() {
        return Err(BoxerError::IsolationViolation(format!(
            "fail-closed: unconfined host execution requires {HOST_UNCONFINED_ACK_ENV}={HOST_UNCONFINED_ACK_VALUE}"
        )));
    }
    if acked {
        Ok(HostModeGuardVerdict {
            allowed: true,
            sandboxed: false,
            warning_banner: Some(
                "AMBER WARNING: Running in explicitly acknowledged unconfined host mode (sandboxed: false)"
                    .to_owned(),
            ),
        })
    } else {
        Ok(HostModeGuardVerdict {
            allowed: true,
            sandboxed: true,
            warning_banner: None,
        })
    }
}

#[must_use]
pub fn has_os_native_sandbox() -> bool {
    binary_exists("bwrap") || binary_exists("unshare") || binary_exists("sandbox-exec")
}
