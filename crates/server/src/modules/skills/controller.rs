//! HTTP endpoints for listing, inspecting, installing, and removing skills.

use std::sync::Arc;

use agents_protocol::{SkillInstallInput, SkillManifest};
use ironic::Json;
use ironic::prelude::*;

use super::services::SkillService;
use crate::infra::error::ServerError;

/// Data transfer object for dynamic skill installation via HTTP.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, garde::Validate, OpenApiSchema)]
pub struct InstallSkillDto {
    #[garde(skip)]
    pub name: Option<String>,
    #[garde(skip)]
    pub description: Option<String>,
    #[garde(skip)]
    pub git_url: Option<String>,
    #[garde(skip)]
    pub git_branch: Option<String>,
    #[garde(skip)]
    pub markdown_content: Option<String>,
    #[garde(skip)]
    pub archive_base64: Option<String>,
}

impl InstallSkillDto {
    /// Converts the DTO into a domain [`SkillInstallInput`].
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] if no source was provided or required fields are missing.
    pub fn into_input(self) -> Result<SkillInstallInput, ServerError> {
        if let Some(url) = self.git_url {
            return Ok(SkillInstallInput::Git {
                url,
                branch: self.git_branch,
            });
        }
        if let Some(content) = self.markdown_content {
            let name = self.name.ok_or_else(|| {
                ServerError::Validation("name is required for markdown skill".into())
            })?;
            return Ok(SkillInstallInput::Markdown {
                name,
                description: self.description,
                content,
            });
        }
        if let Some(data_base64) = self.archive_base64 {
            let name = self.name.ok_or_else(|| {
                ServerError::Validation("name is required for archive skill".into())
            })?;
            return Ok(SkillInstallInput::Archive { name, data_base64 });
        }
        Err(ServerError::Validation(
            "one of git_url, markdown_content, or archive_base64 must be provided".into(),
        ))
    }
}

/// Response payload confirming skill uninstallation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, OpenApiSchema)]
pub struct SkillDeleteResponse {
    pub status: String,
    #[serde(rename = "skillId")]
    pub skill_id: String,
}

#[controller("/api/v1/skills")]
#[derive(Injectable)]
pub struct SkillController {
    service: Arc<SkillService>,
}

#[routes]
impl SkillController {
    #[get]
    #[api(summary = "List all available and built-in skills", tag = "Skills")]
    #[resp(200, "List of skills", json = Vec<SkillManifest>)]
    pub async fn list(&self) -> Result<Json<Vec<SkillManifest>>, HttpError> {
        let skills = self.service.list().await;
        Ok(Json(skills))
    }

    #[get("/:skill_id")]
    #[api(summary = "Get skill manifest by ID", tag = "Skills")]
    #[resp(200, "Skill manifest", json = SkillManifest)]
    pub async fn get(&self, #[param] skill_id: String) -> Result<Json<SkillManifest>, HttpError> {
        let skill = self
            .service
            .get(&skill_id)
            .await
            .ok_or_else(|| ServerError::NotFound(format!("skill `{skill_id}` not found")))?;
        Ok(Json(skill))
    }

    #[post("/install")]
    #[api(
        summary = "Install skill dynamically from Git, Markdown, or Archive",
        tag = "Skills"
    )]
    #[body(json = InstallSkillDto)]
    #[resp(200, "Installed skill manifest", json = SkillManifest)]
    pub async fn install(
        &self,
        #[body] dto: InstallSkillDto,
    ) -> Result<Json<SkillManifest>, HttpError> {
        let input = dto.into_input()?;
        let installed = self.service.install(input).await?;
        Ok(Json(installed))
    }

    #[delete("/:skill_id")]
    #[api(summary = "Uninstall a user-installed skill", tag = "Skills")]
    #[resp(200, "Skill uninstallation status", json = SkillDeleteResponse)]
    pub async fn delete(
        &self,
        #[param] skill_id: String,
    ) -> Result<Json<SkillDeleteResponse>, HttpError> {
        self.service.delete(&skill_id).await?;
        Ok(Json(SkillDeleteResponse {
            status: "deleted".into(),
            skill_id,
        }))
    }
}
