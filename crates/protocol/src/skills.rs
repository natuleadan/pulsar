//! Skill definitions, built-in system skills, and dynamic installation contracts.

use serde::{Deserialize, Serialize};

/// Maximum percentage of the model context window permitted for catalog injection.
pub const MAX_SKILL_BUDGET_PERCENT: f32 = 0.02;

/// Character limit for skill description in catalog injection to preserve context budget.
pub const MAX_DESCRIPTION_LENGTH: usize = 150;

/// Canonical manifest of an installed or built-in skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub built_in: bool,
    pub source: SkillSource,
    pub instructions: String,
    pub tags: Vec<String>,
    pub installed_at_ms: u64,
}

/// Provenance and acquisition source of a skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SkillSource {
    BuiltIn,
    Git {
        url: String,
        branch: Option<String>,
        commit: Option<String>,
    },
    Markdown {
        raw_markdown: String,
    },
    Archive {
        archive_name: String,
        checksum_sha256: String,
    },
}

/// Request to install a dynamic skill from git, markdown, or archive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SkillInstallInput {
    Git {
        url: String,
        branch: Option<String>,
    },
    Markdown {
        name: String,
        description: Option<String>,
        content: String,
    },
    Archive {
        name: String,
        data_base64: String,
    },
}

/// Minimal metadata payload used for context-budget-conscious prompt injection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub built_in: bool,
    pub estimated_tokens: usize,
}

impl SkillCatalogItem {
    #[must_use]
    pub fn from_manifest(manifest: &SkillManifest) -> Self {
        let desc = if manifest.description.len() > MAX_DESCRIPTION_LENGTH {
            format!("{}...", &manifest.description[..MAX_DESCRIPTION_LENGTH - 3])
        } else {
            manifest.description.clone()
        };
        let token_estimate = (manifest.name.len() + desc.len() + 10) / 4;
        Self {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            description: desc,
            built_in: manifest.built_in,
            estimated_tokens: token_estimate.max(5),
        }
    }
}

/// Formats the skills catalog for system prompt injection while enforcing the 2% token budget.
///
/// # Errors
///
/// Returns an error message if the total catalog exceeds the allocated budget.
pub fn format_prompt_catalog(
    items: &[SkillCatalogItem],
    model_context_window: usize,
) -> Result<String, String> {
    let budget_tokens = ((model_context_window as f32) * MAX_SKILL_BUDGET_PERCENT) as usize;
    let total_estimated: usize = items.iter().map(|item| item.estimated_tokens).sum();

    if total_estimated > budget_tokens {
        return Err(format!(
            "skills catalog exceeds 2% context budget: estimated {total_estimated} tokens, allowed {budget_tokens}"
        ));
    }

    let mut out = String::from("Available Skills:\n");
    for item in items {
        out.push_str(&format!("- **{}**: {}\n", item.name, item.description));
    }
    Ok(out)
}

/// Security violation detected during static skill scanning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSecurityViolation {
    pub rule: String,
    pub snippet: String,
    pub description: String,
}

/// Scans raw skill instructions/scripts for security vulnerabilities and forbidden patterns.
///
/// # Errors
///
/// Returns a list of violations if dangerous patterns or leaked credentials are detected.
pub fn scan_skill_security(content: &str) -> Result<(), Vec<SkillSecurityViolation>> {
    let mut violations = Vec::new();

    let forbidden_patterns = [
        (
            "sudo",
            "Elevation with sudo is strictly forbidden (Rule G-6)",
        ),
        (
            "brew install",
            "Package manager invocation is forbidden (Rule G-6)",
        ),
        ("curl | sh", "Unverified remote script pipe is forbidden"),
        ("curl | bash", "Unverified remote bash pipe is forbidden"),
        ("wget | sh", "Unverified remote wget pipe is forbidden"),
        ("wget | bash", "Unverified remote bash pipe is forbidden"),
        ("/usr/local/bin", "Modification of host shims is forbidden"),
        ("chmod 777", "Permissive permission alteration is forbidden"),
    ];

    for (pattern, desc) in forbidden_patterns {
        if content.contains(pattern) {
            violations.push(SkillSecurityViolation {
                rule: "forbidden_command".into(),
                snippet: pattern.into(),
                description: desc.into(),
            });
        }
    }

    let credential_patterns = [
        "AKIA",
        "ghp_",
        "gho_",
        "sk-ant-",
        "sk-proj-",
        "-----BEGIN RSA PRIVATE KEY-----",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
    ];

    for pattern in credential_patterns {
        if content.contains(pattern) {
            violations.push(SkillSecurityViolation {
                rule: "embedded_secret".into(),
                snippet: pattern.into(),
                description: "Embedded secret or private key detected in skill payload".into(),
            });
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

// ---------------------------------------------------------------------------
// 4 Built-in Core System Skills (Hardcoded in strict English for internal agents)
// ---------------------------------------------------------------------------

pub const BUILTIN_SKILL_CORE_TERMINAL: &str = r#"---
name: core-terminal
description: Sanitized terminal execution, process supervision, pipes, and non-zero exit validation.
---
# Core Terminal Skill
Provides controlled process execution inside Boxer sandboxes:
1. Command execution: Strictly within container process group.
2. Exit code audit: Every command must check status; failures output diagnostics.
3. Streamed I/O: Real-time stdout/stderr capture without truncation.
4. Process termination: Respect SIGTERM -> SIGKILL escalation deadlines."#;

pub const BUILTIN_SKILL_FILESYSTEM_VFS: &str = r#"---
name: filesystem-vfs
description: Confined VFS file read, write, patch application, and directory traversal defense.
---
# Filesystem VFS Skill
Enforces confined file access within the allocated task workspace:
1. Strict boundary: All file read/write operations must stay inside workspace root.
2. Traversal rejection: Path components containing `..` or absolute paths are denied.
3. Atomic updates: Modifying files requires verification of clean targets.
4. Scratch hygiene: Ephemeral output files are recorded and cleaned upon exit."#;

pub const BUILTIN_SKILL_GIT_VCS: &str = r#"---
name: git-vcs
description: Git version control, task worktree branching, clean commit discipline, and diff checks.
---
# Git VCS Skill
Automates version control actions conforming to repository standards:
1. Conventional commits: Follow `type(scope): description` with bullet details (Rule G-13).
2. Worktree isolation: Task development runs in dedicated worktrees or feature branches.
3. Parity validation: Checks run cleanly before any commit or pull request.
4. No secret leakage: Prohibits committing `.env` or credential files."#;

pub const BUILTIN_SKILL_CODEBASE_INSPECTOR: &str = r#"---
name: codebase-inspector
description: Architectural codebase analysis, AST symbol discovery, and file length compliance.
---
# Codebase Inspector Skill
Explores and audits project source trees cleanly:
1. Structural mapping: Locates crates, modules, and exported symbol contracts.
2. Size guard: Enforces file length limits (strictly <= 500 lines per file, Rule G-20).
3. Dependency hierarchy: Unidirectional architecture without circular imports.
4. Non-intrusive: Read-only exploration without mutating active source trees."#;

/// Returns the 4 default built-in skills bundled in the system.
#[must_use]
pub fn get_builtin_system_skills() -> Vec<SkillManifest> {
    vec![
        SkillManifest {
            id: "core-terminal".into(),
            name: "core-terminal".into(),
            description: "Sanitized terminal execution, process supervision, pipes, and exit code validation.".into(),
            version: "1.0.0".into(),
            built_in: true,
            source: SkillSource::BuiltIn,
            instructions: BUILTIN_SKILL_CORE_TERMINAL.into(),
            tags: vec!["system".into(), "terminal".into(), "execution".into()],
            installed_at_ms: 0,
        },
        SkillManifest {
            id: "filesystem-vfs".into(),
            name: "filesystem-vfs".into(),
            description: "Confined VFS file read, write, patch application, and directory traversal defense.".into(),
            version: "1.0.0".into(),
            built_in: true,
            source: SkillSource::BuiltIn,
            instructions: BUILTIN_SKILL_FILESYSTEM_VFS.into(),
            tags: vec!["system".into(), "vfs".into(), "storage".into()],
            installed_at_ms: 0,
        },
        SkillManifest {
            id: "git-vcs".into(),
            name: "git-vcs".into(),
            description: "Git version control, task worktree branching, clean commit discipline, and diff checks.".into(),
            version: "1.0.0".into(),
            built_in: true,
            source: SkillSource::BuiltIn,
            instructions: BUILTIN_SKILL_GIT_VCS.into(),
            tags: vec!["system".into(), "git".into(), "vcs".into()],
            installed_at_ms: 0,
        },
        SkillManifest {
            id: "codebase-inspector".into(),
            name: "codebase-inspector".into(),
            description: "Architectural codebase analysis, AST symbol discovery, and file length compliance.".into(),
            version: "1.0.0".into(),
            built_in: true,
            source: SkillSource::BuiltIn,
            instructions: BUILTIN_SKILL_CODEBASE_INSPECTOR.into(),
            tags: vec!["system".into(), "analysis".into(), "ast".into()],
            installed_at_ms: 0,
        },
    ]
}

#[cfg(feature = "openapi")]
impl ironic::OpenApiSchema for SkillManifest {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "SkillManifest",
            "required": ["id", "name", "description", "version", "builtIn", "source", "instructions", "tags", "installedAtMs"],
            "properties": {
                "id": { "type": "string", "example": "core-terminal" },
                "name": { "type": "string", "example": "core-terminal" },
                "description": { "type": "string", "example": "Essential shell execution capabilities" },
                "version": { "type": "string", "example": "1.0.0" },
                "builtIn": { "type": "boolean", "example": true },
                "source": { "type": "string", "example": "builtin" },
                "instructions": { "type": "string", "example": "# Core Terminal..." },
                "tags": { "type": "array", "items": { "type": "string" } },
                "installedAtMs": { "type": "integer", "example": 1731600000000_i64 }
            }
        })
    }
}
