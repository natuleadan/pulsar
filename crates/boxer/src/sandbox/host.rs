//! Host runtime driver with curated env scrubbing, scoped VFS confinement,
//! OS process-group isolation (`process_group(0)`), and `DropGuard` SIGTERM->SIGKILL cleanup.

use std::{
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use pulsar_protocol::{IsolationKind, TaskStreamEvent};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::{mpsc, watch},
};

use crate::{
    config::now_ms,
    error::BoxerError,
    leases::{CleanupDropGuard, SandboxResources, guard::terminate_process_group_sync},
    sandbox::{
        BoxFuture, SandboxDriver, SandboxExecOutput, SandboxExecRequest,
        native_backends::generate_macos_seatbelt_profile,
        policy::{
            binary_exists, build_sanitized_env, resolve_confined_workdir,
            validate_runtime_isolation,
        },
    },
};

/// Executes tasks on the host OS within a strict confined workspace directory.
#[derive(Debug, Clone)]
pub struct HostRuntimeDriver {
    workspace_root: PathBuf,
    strict_native_isolation: bool,
}

impl HostRuntimeDriver {
    #[must_use]
    pub fn new(workspace_root: PathBuf, strict_native_isolation: bool) -> Self {
        Self {
            workspace_root,
            strict_native_isolation,
        }
    }
}

impl SandboxDriver for HostRuntimeDriver {
    fn execute<'a>(
        &'a self,
        req: SandboxExecRequest,
        stream_tx: Option<mpsc::Sender<TaskStreamEvent>>,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<SandboxExecOutput, BoxerError>> {
        Box::pin(async move {
            validate_runtime_isolation(&req.profile, self.strict_native_isolation)?;

            let workdir = resolve_confined_workdir(
                &self.workspace_root,
                &req.session_id,
                req.exec.working_dir.as_deref(),
            )?;

            let env_map = build_sanitized_env(&req.exec, req.proxy_url.as_deref());
            let timeout = Duration::from_secs(req.exec.timeout_seconds.unwrap_or(120).max(1));

            let mut cmd = build_host_command(&req, &workdir);
            cmd.current_dir(&workdir)
                .env_clear()
                .envs(env_map)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .kill_on_drop(true);

            let mut child = cmd.spawn().map_err(|err| {
                BoxerError::ExecutionFailed(format!("failed to spawn host sandbox process: {err}"))
            })?;

            let pgid = child.id();
            let mut resources = SandboxResources::new(&req.task_id);
            resources.process_group_id = pgid;
            let _guard = CleanupDropGuard::new(resources.clone(), |res| res.cleanup_sync());

            let stdout_pipe = child.stdout.take();
            let stderr_pipe = child.stderr.take();
            let seq = Arc::new(AtomicU64::new(1));

            let stdout_handle = tokio::spawn(drain_stream(
                req.task_id.clone(),
                stdout_pipe,
                false,
                stream_tx.clone(),
                Arc::clone(&seq),
            ));
            let stderr_handle = tokio::spawn(drain_stream(
                req.task_id.clone(),
                stderr_pipe,
                true,
                stream_tx,
                seq,
            ));

            let mut timed_out = false;
            let mut cancelled = false;
            let status_code = tokio::select! {
                wait_res = child.wait() => {
                    wait_res
                        .map_err(|err| BoxerError::ExecutionFailed(format!("wait failed: {err}")))?
                        .code()
                        .unwrap_or(-1)
                }
                _ = tokio::time::sleep(timeout) => {
                    timed_out = true;
                    if let Some(pid) = pgid {
                        terminate_process_group_sync(pid);
                    }
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    -15
                }
                is_cancelled = async { cancel_rx.wait_for(|v| *v).await.is_ok() } => {
                    if is_cancelled {
                        cancelled = true;
                        if let Some(pid) = pgid {
                            terminate_process_group_sync(pid);
                        }
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        -15
                    } else {
                        child
                            .wait()
                            .await
                            .map_err(|err| BoxerError::ExecutionFailed(format!("wait failed: {err}")))?
                            .code()
                            .unwrap_or(-1)
                    }
                }
            };

            let stdout = stdout_handle.await.unwrap_or_default();
            let stderr = stderr_handle.await.unwrap_or_default();

            if timed_out {
                return Ok(SandboxExecOutput {
                    exit_code: status_code,
                    stdout,
                    stderr,
                    timed_out: true,
                    cancelled: false,
                    working_dir: workdir,
                });
            }
            if cancelled {
                return Ok(SandboxExecOutput {
                    exit_code: status_code,
                    stdout,
                    stderr,
                    timed_out: false,
                    cancelled: true,
                    working_dir: workdir,
                });
            }

            Ok(SandboxExecOutput {
                exit_code: status_code,
                stdout,
                stderr,
                timed_out: false,
                cancelled: false,
                working_dir: workdir,
            })
        })
    }
}

fn build_host_command(req: &SandboxExecRequest, workdir: &std::path::Path) -> Command {
    let wrapped_script = match req.exec.max_memory_mb {
        Some(mb) if mb > 0 => {
            let kb = mb.saturating_mul(1024);
            format!("ulimit -v {kb} >/dev/null 2>&1 || true; {}", req.command)
        }
        _ => req.command.clone(),
    };

    if req.profile.isolation == IsolationKind::Native && binary_exists("bwrap") {
        let mut cmd = Command::new("bwrap");
        let workdir_str = workdir.to_string_lossy().into_owned();
        let mut args = vec![
            "--die-with-parent".to_owned(),
            "--unshare-user".to_owned(),
            "--unshare-pid".to_owned(),
            "--unshare-net".to_owned(),
            "--ro-bind".to_owned(),
            "/usr".to_owned(),
            "/usr".to_owned(),
            "--ro-bind".to_owned(),
            "/bin".to_owned(),
            "/bin".to_owned(),
            "--ro-bind".to_owned(),
            "/lib".to_owned(),
            "/lib".to_owned(),
            "--bind".to_owned(),
            workdir_str.clone(),
            workdir_str.clone(),
            "--chdir".to_owned(),
            workdir_str,
            "--proc".to_owned(),
            "/proc".to_owned(),
            "--dev".to_owned(),
            "/dev".to_owned(),
            "--tmpfs".to_owned(),
            "/tmp".to_owned(),
        ];
        args.extend(["sh".to_owned(), "-c".to_owned(), wrapped_script]);
        cmd.args(args);
        return cmd;
    }

    if req.profile.isolation == IsolationKind::Native
        && cfg!(target_os = "macos")
        && binary_exists("sandbox-exec")
    {
        let profile = generate_macos_seatbelt_profile(workdir, req.exec.network_egress);
        let mut cmd = Command::new("sandbox-exec");
        cmd.args(["-p", &profile.scheme_source, "sh", "-c", &wrapped_script]);
        return cmd;
    }

    let mut cmd = Command::new("sh");
    cmd.args(["-c", &wrapped_script]);
    cmd
}

async fn drain_stream<R: tokio::io::AsyncRead + Unpin>(
    task_id: String,
    reader: Option<R>,
    is_stderr: bool,
    tx: Option<mpsc::Sender<TaskStreamEvent>>,
    seq: Arc<AtomicU64>,
) -> String {
    let Some(r) = reader else {
        return String::new();
    };
    let mut lines = BufReader::new(r).lines();
    let mut collected = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if !collected.is_empty() {
            collected.push('\n');
        }
        collected.push_str(&line);
        if let Some(sender) = &tx {
            let ev = TaskStreamEvent {
                task_id: task_id.clone(),
                seq: seq.fetch_add(1, Ordering::SeqCst),
                emitted_at_ms: now_ms(),
                chunk: line,
                is_stderr,
            };
            let _ = sender.send(ev).await;
        }
    }
    collected
}
