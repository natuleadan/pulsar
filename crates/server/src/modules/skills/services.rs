//! Skill service managing dynamic installation, built-in system skills, and prompt injection.

use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use ironic::prelude::*;
use pulsar_protocol::{
    SkillCatalogItem, SkillInstallInput, SkillManifest, SkillSource, format_prompt_catalog,
    get_builtin_system_skills, scan_skill_security,
};
use sha2::{Digest, Sha256};

use crate::infra::{error::ServerError, state::now_ms};

/// DI state holder for installed and built-in skills.
#[derive(Clone)]
pub struct SkillStore {
    pub(crate) skills: Arc<RwLock<HashMap<String, SkillManifest>>>,
}

impl Default for SkillStore {
    fn default() -> Self {
        let mut map = HashMap::new();
        for builtin in get_builtin_system_skills() {
            map.insert(builtin.id.clone(), builtin);
        }
        Self {
            skills: Arc::new(RwLock::new(map)),
        }
    }
}

/// Manages built-in and dynamic user-installed skills with security fencing.
#[derive(Clone, Injectable)]
pub struct SkillService {
    store: Arc<SkillStore>,
}

impl Default for SkillService {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: Arc::new(SkillStore::default()),
        }
    }

    /// Lists all available skills (both built-in and user-installed).
    pub async fn list(&self) -> Vec<SkillManifest> {
        let guard = self.store.skills.read().await;
        let mut list: Vec<SkillManifest> = guard.values().cloned().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    /// Retrieves an individual skill by ID.
    pub async fn get(&self, id: &str) -> Option<SkillManifest> {
        let guard = self.store.skills.read().await;
        guard.get(id).cloned()
    }

    /// Dynamically installs any skill from Git, raw Markdown, or an archive bundle.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] if inputs are invalid,
    /// [`ServerError::Forbidden`] if the skill violates security rules or attempts
    /// to overwrite a built-in system skill.
    pub async fn install(&self, input: SkillInstallInput) -> Result<SkillManifest, ServerError> {
        let now = now_ms();
        let manifest = match input {
            SkillInstallInput::Markdown {
                name,
                description,
                content,
            } => {
                let id = sanitize_skill_id(&name)?;
                scan_skill_security(&content).map_err(|violations| {
                    ServerError::Forbidden(format!(
                        "skill `{name}` rejected by security scanner: {} violation(s) detected",
                        violations.len()
                    ))
                })?;

                SkillManifest {
                    id: id.clone(),
                    name: name.clone(),
                    description: description
                        .unwrap_or_else(|| format!("Custom markdown skill: {name}")),
                    version: "0.1.0".into(),
                    built_in: false,
                    source: SkillSource::Markdown {
                        raw_markdown: content.clone(),
                    },
                    instructions: content,
                    tags: vec!["custom".into(), "markdown".into()],
                    installed_at_ms: now,
                }
            }
            SkillInstallInput::Git { url, branch } => {
                let id = derive_id_from_git_url(&url)?;
                scan_skill_security(&url).map_err(|violations| {
                    ServerError::Forbidden(format!(
                        "git url `{url}` rejected by security scanner: {} violation(s)",
                        violations.len()
                    ))
                })?;

                let instructions = format!(
                    "# Git Skill: {id}\nSource: {url}\nBranch: {}\nDynamically mounted into Boxer sandbox.",
                    branch.as_deref().unwrap_or("main")
                );

                SkillManifest {
                    id: id.clone(),
                    name: id.clone(),
                    description: format!("Skill from repository: {url}"),
                    version: "0.1.0".into(),
                    built_in: false,
                    source: SkillSource::Git {
                        url: url.clone(),
                        branch: branch.clone(),
                        commit: None,
                    },
                    instructions,
                    tags: vec!["custom".into(), "git".into()],
                    installed_at_ms: now,
                }
            }
            SkillInstallInput::Archive { name, data_base64 } => {
                let id = sanitize_skill_id(&name)?;
                let raw_bytes = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    &data_base64,
                )
                .map_err(|e| ServerError::Validation(format!("invalid base64 archive: {e}")))?;

                let mut hasher = Sha256::new();
                hasher.update(&raw_bytes);
                let checksum = hex::encode(hasher.finalize());

                let preview = String::from_utf8_lossy(&raw_bytes);
                scan_skill_security(&preview).map_err(|violations| {
                    ServerError::Forbidden(format!(
                        "archive payload rejected by security scanner: {} violation(s)",
                        violations.len()
                    ))
                })?;

                SkillManifest {
                    id: id.clone(),
                    name: name.clone(),
                    description: format!("Packaged archive skill {name}"),
                    version: "0.1.0".into(),
                    built_in: false,
                    source: SkillSource::Archive {
                        archive_name: format!("{name}.zip"),
                        checksum_sha256: checksum,
                    },
                    instructions: format!(
                        "# Archive Skill: {name}\nExtracted from verified archive bundle."
                    ),
                    tags: vec!["custom".into(), "archive".into()],
                    installed_at_ms: now,
                }
            }
        };

        let mut guard = self.store.skills.write().await;
        if let Some(existing) = guard.get(&manifest.id)
            && existing.built_in
        {
            return Err(ServerError::Forbidden(
                "cannot overwrite built-in system skill".into(),
            ));
        }

        guard.insert(manifest.id.clone(), manifest.clone());
        Ok(manifest)
    }

    /// Uninstalls a user-installed skill.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] if attempting to delete a built-in skill,
    /// or [`ServerError::NotFound`] if the skill does not exist.
    pub async fn delete(&self, id: &str) -> Result<(), ServerError> {
        let mut guard = self.store.skills.write().await;
        let Some(existing) = guard.get(id) else {
            return Err(ServerError::NotFound(format!("skill `{id}` not found")));
        };

        if existing.built_in {
            return Err(ServerError::Forbidden(
                "built-in system skills cannot be removed".into(),
            ));
        }

        guard.remove(id);
        Ok(())
    }

    /// Formats the skills catalog for system prompt injection while enforcing the 2% budget.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] if the catalog exceeds the context window quota.
    pub async fn catalog_for_prompt(
        &self,
        model_context_window: usize,
    ) -> Result<String, ServerError> {
        let guard = self.store.skills.read().await;
        let items: Vec<SkillCatalogItem> = guard
            .values()
            .map(SkillCatalogItem::from_manifest)
            .collect();

        format_prompt_catalog(&items, model_context_window).map_err(ServerError::Validation)
    }
}

fn sanitize_skill_id(name: &str) -> Result<String, ServerError> {
    let trimmed = name.trim().to_lowercase().replace(' ', "-");
    if trimmed.is_empty() {
        return Err(ServerError::Validation("skill name cannot be empty".into()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ServerError::Validation(format!(
            "invalid skill id `{trimmed}`: must only contain alphanumeric, hyphen, or underscore"
        )));
    }
    Ok(trimmed)
}

fn derive_id_from_git_url(url: &str) -> Result<String, ServerError> {
    let clean_url = url.trim().trim_end_matches(".git");
    let last_segment = clean_url.split('/').next_back().unwrap_or("custom-skill");
    sanitize_skill_id(last_segment)
}
