use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::error::BoxerError;

pub const MAX_PENDING_MUTATIONS_PER_FILE: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationQueueError {
    FileResourceBusy {
        realpath: String,
        queued_count: usize,
    },
    ConflictStaleBase {
        realpath: String,
        expected_sha256: String,
        actual_sha256: String,
    },
    IoError(String),
}

impl std::fmt::Display for MutationQueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileResourceBusy {
                realpath,
                queued_count,
            } => {
                write!(
                    f,
                    "file_resource_busy: `{realpath}` queue saturated ({queued_count} waiting)"
                )
            }
            Self::ConflictStaleBase {
                realpath,
                expected_sha256,
                actual_sha256,
            } => {
                write!(
                    f,
                    "conflict_stale_base: `{realpath}` expected `{expected_sha256}`, found `{actual_sha256}`"
                )
            }
            Self::IoError(msg) => write!(f, "mutation io error: {msg}"),
        }
    }
}

impl std::error::Error for MutationQueueError {}

impl From<MutationQueueError> for BoxerError {
    fn from(value: MutationQueueError) -> Self {
        Self::ExecutionFailed(value.to_string())
    }
}

struct FileSlot {
    lock: Arc<Mutex<()>>,
    waiting: Arc<AtomicUsize>,
}

#[derive(Clone, Default)]
pub struct FileMutationQueue {
    slots: Arc<Mutex<HashMap<PathBuf, FileSlot>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMutationReceipt {
    pub realpath: PathBuf,
    pub previous_sha256: String,
    pub new_sha256: String,
    pub bytes_written: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReconciliationState {
    pub realpath: PathBuf,
    pub exists: bool,
    pub current_sha256: String,
    pub size_bytes: u64,
}

impl FileMutationQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn mutate_optimistic(
        &self,
        target_path: &Path,
        expected_base_sha256: Option<&str>,
        new_content: &[u8],
    ) -> Result<FileMutationReceipt, MutationQueueError> {
        let canonical = resolve_or_init_realpath(target_path)?;
        let (file_lock, waiting) = {
            let mut map = self.slots.lock().await;
            let entry = map.entry(canonical.clone()).or_insert_with(|| FileSlot {
                lock: Arc::new(Mutex::new(())),
                waiting: Arc::new(AtomicUsize::new(0)),
            });
            let current_waiting = entry.waiting.load(Ordering::SeqCst);
            if current_waiting >= MAX_PENDING_MUTATIONS_PER_FILE {
                return Err(MutationQueueError::FileResourceBusy {
                    realpath: canonical.display().to_string(),
                    queued_count: current_waiting,
                });
            }
            entry.waiting.fetch_add(1, Ordering::SeqCst);
            (Arc::clone(&entry.lock), Arc::clone(&entry.waiting))
        };

        let _guard = file_lock.lock().await;
        let result = async {
            let existing_bytes = tokio::fs::read(&canonical).await.unwrap_or_default();
            let actual_sha = sha256_hex(&existing_bytes);

            if let Some(expected) = expected_base_sha256
                && !expected.eq_ignore_ascii_case(&actual_sha)
            {
                return Err(MutationQueueError::ConflictStaleBase {
                    realpath: canonical.display().to_string(),
                    expected_sha256: expected.to_owned(),
                    actual_sha256: actual_sha,
                });
            }

            tokio::fs::write(&canonical, new_content)
                .await
                .map_err(|e| MutationQueueError::IoError(e.to_string()))?;
            let new_sha = sha256_hex(new_content);

            Ok(FileMutationReceipt {
                realpath: canonical,
                previous_sha256: actual_sha,
                new_sha256: new_sha,
                bytes_written: new_content.len(),
            })
        }
        .await;

        waiting.fetch_sub(1, Ordering::SeqCst);
        result
    }

    pub async fn reconcile_file_state(
        &self,
        target_path: &Path,
    ) -> Result<FileReconciliationState, MutationQueueError> {
        let canonical = resolve_or_init_realpath(target_path)?;
        match tokio::fs::read(&canonical).await {
            Ok(bytes) => Ok(FileReconciliationState {
                realpath: canonical,
                exists: true,
                current_sha256: sha256_hex(&bytes),
                size_bytes: bytes.len() as u64,
            }),
            Err(_) => Ok(FileReconciliationState {
                realpath: canonical,
                exists: false,
                current_sha256: sha256_hex(b""),
                size_bytes: 0,
            }),
        }
    }
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn resolve_or_init_realpath(target_path: &Path) -> Result<PathBuf, MutationQueueError> {
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| MutationQueueError::IoError(e.to_string()))?;
    }
    if !target_path.exists() {
        std::fs::write(target_path, b"").map_err(|e| MutationQueueError::IoError(e.to_string()))?;
    }
    std::fs::canonicalize(target_path).map_err(|e| MutationQueueError::IoError(e.to_string()))
}
