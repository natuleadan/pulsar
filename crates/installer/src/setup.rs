//! Zero-sudo user-space installation, container runtime detection, and setup modes.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// High-level installation and operational setup mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalSetupMode {
    /// Standalone mode executing sandboxed tasks directly on the local machine.
    LocalAutonomous,
    /// Edge mode routing tasks to a remote sovereign orchestrator VPS.
    RemoteOrchestrator,
}

impl LocalSetupMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalAutonomous => "local_autonomous",
            Self::RemoteOrchestrator => "remote_orchestrator",
        }
    }
}

/// Information about a detected local container runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedRuntime {
    pub name: String,
    pub available: bool,
    pub socket_path: Option<String>,
    pub guidance: String,
}

/// Resolves standard zero-sudo user-space installation paths under `$HOME`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSpaceInstallPaths {
    pub bin_dir: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Default for UserSpaceInstallPaths {
    fn default() -> Self {
        Self::resolve_from_home(None)
    }
}

impl UserSpaceInstallPaths {
    #[must_use]
    pub fn resolve_from_home(home_override: Option<&Path>) -> Self {
        let home = home_override
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));

        Self {
            bin_dir: home.join(".local").join("bin"),
            config_dir: home.join(".config").join("pulsar"),
            data_dir: home.join(".local").join("share").join("pulsar"),
        }
    }
}

/// Detects local container engines (Docker, Colima, Podman) without requiring sudo privileges.
#[must_use]
pub fn detect_local_container_runtimes() -> Vec<DetectedRuntime> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut list = Vec::new();

    // 1. Check Docker user socket
    let docker_sock_user = home.join(".docker").join("run").join("docker.sock");
    let docker_sock_system = PathBuf::from("/var/run/docker.sock");
    let (docker_avail, docker_sock) = if docker_sock_user.exists() {
        (true, Some(docker_sock_user.to_string_lossy().to_string()))
    } else if docker_sock_system.exists() {
        (true, Some(docker_sock_system.to_string_lossy().to_string()))
    } else {
        (false, None)
    };

    list.push(DetectedRuntime {
        name: "Docker".into(),
        available: docker_avail,
        socket_path: docker_sock,
        guidance: if docker_avail {
            "Docker socket active and available for containerized tasks.".into()
        } else {
            "Install Docker Desktop or start Docker service to enable container sandboxing.".into()
        },
    });

    // 2. Check Colima user socket
    let colima_sock = home.join(".colima").join("default").join("docker.sock");
    let colima_avail = colima_sock.exists();
    list.push(DetectedRuntime {
        name: "Colima".into(),
        available: colima_avail,
        socket_path: if colima_avail {
            Some(colima_sock.to_string_lossy().to_string())
        } else {
            None
        },
        guidance: if colima_avail {
            "Colima minimal container runtime active.".into()
        } else {
            "Run `colima start` if using Colima as a lightweight Docker alternative.".into()
        },
    });

    // 3. Check Podman rootless socket
    let podman_sock = home
        .join(".local")
        .join("share")
        .join("containers")
        .join("podman")
        .join("machine")
        .join("podman.sock");
    let podman_avail = podman_sock.exists();
    list.push(DetectedRuntime {
        name: "Podman (Rootless)".into(),
        available: podman_avail,
        socket_path: if podman_avail {
            Some(podman_sock.to_string_lossy().to_string())
        } else {
            None
        },
        guidance: if podman_avail {
            "Podman rootless engine detected.".into()
        } else {
            "Podman rootless engine is supported for unprivileged environments.".into()
        },
    });

    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_space_install_paths_resolution() {
        let fake_home = Path::new("/tmp/fake_home");
        let paths = UserSpaceInstallPaths::resolve_from_home(Some(fake_home));

        assert_eq!(paths.bin_dir, fake_home.join(".local/bin"));
        assert_eq!(paths.config_dir, fake_home.join(".config/pulsar"));
        assert_eq!(paths.data_dir, fake_home.join(".local/share/pulsar"));

        // Must NEVER point to system folders requiring sudo
        assert!(!paths.bin_dir.starts_with("/usr/local/bin"));
        assert!(!paths.bin_dir.starts_with("/etc"));
    }

    #[test]
    fn test_detect_runtimes_returns_structured_matrix() {
        let runtimes = detect_local_container_runtimes();
        assert_eq!(runtimes.len(), 3);
        assert!(runtimes.iter().any(|r| r.name == "Docker"));
        assert!(runtimes.iter().any(|r| r.name == "Colima"));
        assert!(runtimes.iter().any(|r| r.name == "Podman (Rootless)"));
    }
}
