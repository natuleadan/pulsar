use std::{collections::HashMap, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{error::BoxerError, runtime::engine::RuntimeTool};

pub const CODEMODE_MAX_MEMORY_BYTES: usize = 128 * 1024 * 1024;
pub const CODEMODE_MAX_CPU_SECS: u64 = 15;
pub const CODEMODE_MAX_OUTPUT_CHARS: usize = 64 * 1024;
pub const CODEMODE_MAX_STATEMENTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodemodeStatement {
    pub step_id: String,
    pub tool_name: String,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodemodeScript {
    pub statements: Vec<CodemodeStatement>,
    #[serde(default)]
    pub simulate_infinite_loop: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodemodeExecutionReport {
    pub steps_executed: usize,
    pub consolidated_output: String,
    pub truncated: bool,
    pub truncation_locator: Option<String>,
    pub timed_out: bool,
}

pub struct SandboxedCodemodeVm {
    tools: HashMap<String, Arc<dyn RuntimeTool>>,
    timeout_duration: Duration,
    max_output_chars: usize,
}

impl Default for SandboxedCodemodeVm {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxedCodemodeVm {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            timeout_duration: Duration::from_secs(CODEMODE_MAX_CPU_SECS),
            max_output_chars: CODEMODE_MAX_OUTPUT_CHARS,
        }
    }

    #[must_use]
    pub fn with_limits(mut self, timeout: Duration, max_output_chars: usize) -> Self {
        self.timeout_duration = timeout;
        self.max_output_chars = max_output_chars.max(64);
        self
    }

    pub fn register_tool(&mut self, tool: Arc<dyn RuntimeTool>) {
        self.tools.insert(tool.name().to_owned(), tool);
    }

    pub async fn execute_script(
        &self,
        script: &CodemodeScript,
    ) -> Result<CodemodeExecutionReport, BoxerError> {
        if script.statements.len() > CODEMODE_MAX_STATEMENTS {
            return Err(BoxerError::ExecutionFailed(format!(
                "codemode script exceeds maximum statement count ({CODEMODE_MAX_STATEMENTS})"
            )));
        }

        let fut = async {
            if script.simulate_infinite_loop {
                tokio::time::sleep(self.timeout_duration + Duration::from_secs(5)).await;
            }
            let mut outputs = Vec::with_capacity(script.statements.len());
            for stmt in &script.statements {
                if stmt.tool_name.starts_with("host_")
                    || stmt.tool_name == "fs_raw"
                    || stmt.tool_name == "net_socket"
                {
                    return Err(BoxerError::IsolationViolation(format!(
                        "codemode VM blocked direct host/network capability `{}`",
                        stmt.tool_name
                    )));
                }
                let Some(tool) = self.tools.get(&stmt.tool_name) else {
                    return Err(BoxerError::ExecutionFailed(format!(
                        "codemode tool `tools.{}` is not registered",
                        stmt.tool_name
                    )));
                };
                let res = tool.execute(stmt.args.clone()).await?;
                outputs.push(json!({
                    "stepId": stmt.step_id,
                    "tool": stmt.tool_name,
                    "result": res,
                }));
            }
            Ok::<(usize, String), BoxerError>((
                outputs.len(),
                serde_json::to_string(&outputs).unwrap_or_default(),
            ))
        };

        match tokio::time::timeout(self.timeout_duration, fut).await {
            Ok(Ok((steps, raw_out))) => {
                if raw_out.len() > self.max_output_chars {
                    let sliced = raw_out[..self.max_output_chars].to_owned();
                    Ok(CodemodeExecutionReport {
                        steps_executed: steps,
                        consolidated_output: sliced,
                        truncated: true,
                        truncation_locator: Some(format!(
                            "codemode://output/truncated?total_bytes={}&limit_bytes={}",
                            raw_out.len(),
                            self.max_output_chars
                        )),
                        timed_out: false,
                    })
                } else {
                    Ok(CodemodeExecutionReport {
                        steps_executed: steps,
                        consolidated_output: raw_out,
                        truncated: false,
                        truncation_locator: None,
                        timed_out: false,
                    })
                }
            }
            Ok(Err(err)) => Err(err),
            Err(_) => Ok(CodemodeExecutionReport {
                steps_executed: 0,
                consolidated_output: String::new(),
                truncated: false,
                truncation_locator: Some("codemode://timeout/terminated".to_owned()),
                timed_out: true,
            }),
        }
    }
}
