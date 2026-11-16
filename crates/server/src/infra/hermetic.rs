//! Hermetic testing harness and sandbox isolation (Fase 4.2).
//!
//! Enforces hermetic isolation rules: tests must never touch the developer's
//! real `$HOME`, SSH keys, or host keychain. Every execution runs within
//! an ephemeral scratch root auto-cleaned upon drop.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::infra::error::ServerError;

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Ephemeral, self-cleaning sandbox environment for hermetic test execution.
#[derive(Debug)]
pub struct HermeticSandbox {
    pub root: PathBuf,
    pub home: PathBuf,
    pub tmp: PathBuf,
    pub scratch: PathBuf,
}

impl HermeticSandbox {
    /// Creates a new hermetic sandbox under the system temp directory.
    pub fn new() -> Result<Self, std::io::Error> {
        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dirname = format!("agents-hermetic-{pid}-{count}");
        let root = std::env::temp_dir().join(dirname);

        let home = root.join("home");
        let tmp = root.join("tmp");
        let scratch = root.join("scratch");

        std::fs::create_dir_all(&home)?;
        std::fs::create_dir_all(&tmp)?;
        std::fs::create_dir_all(&scratch)?;

        Ok(Self {
            root,
            home,
            tmp,
            scratch,
        })
    }

    /// Verifies whether `path` is strictly contained within the hermetic sandbox root.
    #[must_use]
    pub fn is_contained(&self, path: &Path) -> bool {
        if let (Ok(canonical_root), Ok(canonical_target)) =
            (self.root.canonicalize(), path.canonicalize())
        {
            canonical_target.starts_with(canonical_root)
        } else {
            path.starts_with(&self.root)
        }
    }

    /// Resolves `relative_subpath` within the sandbox scratch area, rejecting traversal.
    pub fn resolve_scratch_path(&self, relative_subpath: &str) -> Result<PathBuf, ServerError> {
        if relative_subpath.contains('\0') {
            return Err(ServerError::Validation(
                "Null bytes are strictly prohibited in paths".into(),
            ));
        }

        let normalized = Path::new(relative_subpath);
        for component in normalized.components() {
            if let std::path::Component::ParentDir = component {
                return Err(ServerError::Validation(
                    "Directory traversal components (..) are rejected".into(),
                ));
            }
        }

        let target = self.scratch.join(normalized);
        Ok(target)
    }
}

impl Drop for HermeticSandbox {
    fn drop(&mut self) {
        if self.root.exists() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// Explicit, thread-safe in-memory environment dictionary isolated from host env.
#[derive(Debug, Clone, Default)]
pub struct HermeticEnvironment {
    vars: BTreeMap<String, String>,
}

impl HermeticEnvironment {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermetic_sandbox_lifecycle_and_traversal_blocking() {
        let sandbox = HermeticSandbox::new().expect("sandbox creation failed");
        assert!(sandbox.root.exists());
        assert!(sandbox.home.exists());
        assert!(sandbox.scratch.exists());

        let valid_file = sandbox
            .resolve_scratch_path("workspace/project.txt")
            .expect("valid path rejected");
        assert!(valid_file.starts_with(&sandbox.scratch));

        let traversal_result = sandbox.resolve_scratch_path("../../../etc/shadow");
        assert!(traversal_result.is_err());

        let null_result = sandbox.resolve_scratch_path("file\0.txt");
        assert!(null_result.is_err());

        let root_copy = sandbox.root.clone();
        drop(sandbox);
        assert!(!root_copy.exists(), "sandbox root must be cleaned on drop");
    }

    #[test]
    fn hermetic_environment_isolation() {
        let mut env = HermeticEnvironment::new();
        env.set("APP_KEY", "isolated-secret");
        assert_eq!(env.get("APP_KEY"), Some("isolated-secret"));
        assert_eq!(env.get("NONEXISTENT"), None);
    }
}
