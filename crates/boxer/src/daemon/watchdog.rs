use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionToolMask {
    pub active_disabled_tools: BTreeSet<String>,
    pub staged_disabled_tools: BTreeSet<String>,
    pub turn_epoch: u64,
}

impl SessionToolMask {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stage_disable_tool(&mut self, tool_name: impl Into<String>) {
        self.staged_disabled_tools
            .insert(tool_name.into().trim().to_ascii_lowercase());
    }

    pub fn stage_enable_tool(&mut self, tool_name: &str) {
        self.staged_disabled_tools
            .remove(&tool_name.trim().to_ascii_lowercase());
    }

    pub fn commit_turn_boundary(&mut self) {
        self.active_disabled_tools = self.staged_disabled_tools.clone();
        self.turn_epoch = self.turn_epoch.saturating_add(1);
    }

    #[must_use]
    pub fn is_tool_enabled_for_current_turn(&self, tool_name: &str) -> bool {
        !self
            .active_disabled_tools
            .contains(&tool_name.trim().to_ascii_lowercase())
    }

    #[must_use]
    pub fn filter_manifest_for_next_turn(&mut self, tools: &[String]) -> Vec<String> {
        self.commit_turn_boundary();
        tools
            .iter()
            .filter(|t| self.is_tool_enabled_for_current_turn(t))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemOneDecisionKind {
    CompilingLongTask,
    WaitingForIo,
    HangingDead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WatchdogAction {
    ContinueNormal,
    ExtendLeaseSecs(u64),
    HardKillSwitch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchdogTelemetrySample {
    pub silence_duration_secs: u64,
    pub cpu_usage_permille: u32,
    pub memory_delta_kb: i64,
    pub last_tool_name: Option<String>,
    pub stdout_tail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemOneVerdict {
    pub decision: SystemOneDecisionKind,
    pub confidence_permille: u16,
    pub action: WatchdogAction,
    pub reason: String,
}

pub const WATCHDOG_SILENCE_THRESHOLD_SECS: u64 = 30;
pub const WATCHDOG_CONFIDENCE_KILL_THRESHOLD_PERMILLE: u16 = 850;

#[must_use]
pub fn evaluate_system_one_watchdog(sample: &WatchdogTelemetrySample) -> SystemOneVerdict {
    if sample.silence_duration_secs < WATCHDOG_SILENCE_THRESHOLD_SECS {
        return SystemOneVerdict {
            decision: SystemOneDecisionKind::WaitingForIo,
            confidence_permille: 990,
            action: WatchdogAction::ContinueNormal,
            reason: "silence below 30s threshold".to_owned(),
        };
    }

    let tail_lower = sample.stdout_tail.to_ascii_lowercase();
    let tool_lower = sample
        .last_tool_name
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();

    let is_compiling = tail_lower.contains("compiling")
        || tail_lower.contains("building")
        || tail_lower.contains("downloading")
        || tail_lower.contains("linking")
        || (sample.cpu_usage_permille >= 150 && sample.memory_delta_kb.abs() > 128);

    if is_compiling {
        return SystemOneVerdict {
            decision: SystemOneDecisionKind::CompilingLongTask,
            confidence_permille: 960,
            action: WatchdogAction::ExtendLeaseSecs(60),
            reason: "active compilation or heavy build progress detected in cgroups/stdout"
                .to_owned(),
        };
    }

    let is_waiting_io = tail_lower.contains("waiting for")
        || tail_lower.contains(" migrating ")
        || tool_lower.contains("fetch")
        || tool_lower.contains("http");

    if is_waiting_io && sample.silence_duration_secs < 90 {
        return SystemOneVerdict {
            decision: SystemOneDecisionKind::WaitingForIo,
            confidence_permille: 910,
            action: WatchdogAction::ExtendLeaseSecs(60),
            reason: "legitimate external I/O wait classified by System 1 evaluator".to_owned(),
        };
    }

    let confidence_permille: u16 = if sample.cpu_usage_permille == 0 && sample.memory_delta_kb == 0
    {
        970
    } else {
        890
    };

    let action = if confidence_permille >= WATCHDOG_CONFIDENCE_KILL_THRESHOLD_PERMILLE {
        WatchdogAction::HardKillSwitch
    } else {
        WatchdogAction::ExtendLeaseSecs(30)
    };

    SystemOneVerdict {
        decision: SystemOneDecisionKind::HangingDead,
        confidence_permille,
        action,
        reason: "30s+ silence with zero progress classified as zombie/hanging process".to_owned(),
    }
}
