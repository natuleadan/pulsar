//! Public programmatic SDK for interacting with `pulsar-server` and `pulsar-boxer`.

#![forbid(unsafe_code)]

use pulsar_protocol::{SkillInstallInput, SkillManifest};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use thiserror::Error;

/// SDK-specific error enum wrapping network and server responses.
#[derive(Debug, Error)]
pub enum SdkError {
    #[error("http client error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("api returned error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("configuration error: {0}")]
    Config(String),
}

/// Client configuration for connecting to an Pulsar Sovereign Node.
#[derive(Debug, Clone)]
pub struct SdkConfig {
    pub base_url: String,
    pub api_token: Option<String>,
}

impl SdkConfig {
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_token: None,
        }
    }

    #[must_use]
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.api_token = Some(token.into());
        self
    }
}

/// Programmatic SDK client for the Pulsar orchestrator platform.
#[derive(Clone)]
pub struct PulsarSdk {
    config: SdkConfig,
    client: reqwest::Client,
}

impl PulsarSdk {
    /// Creates a new SDK instance from a [`SdkConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::Config`] if HTTP client construction fails.
    pub fn new(config: SdkConfig) -> Result<Self, SdkError> {
        let mut headers = HeaderMap::new();
        if let Some(token) = &config.api_token {
            let mut val = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| SdkError::Config(format!("invalid token header: {e}")))?;
            val.set_sensitive(true);
            headers.insert(AUTHORIZATION, val);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        Ok(Self { config, client })
    }

    /// Fetches server health status.
    pub async fn health(&self) -> Result<serde_json::Value, SdkError> {
        let url = format!("{}/health", self.config.base_url);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(SdkError::Api {
                status: resp.status().as_u16(),
                message: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(resp.json().await?)
    }

    /// Lists all installed and built-in skills.
    pub async fn list_skills(&self) -> Result<Vec<SkillManifest>, SdkError> {
        let url = format!("{}/api/v1/skills", self.config.base_url);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(SdkError::Api {
                status: resp.status().as_u16(),
                message: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(resp.json().await?)
    }

    /// Retrieves an individual skill manifest by ID.
    pub async fn get_skill(&self, skill_id: &str) -> Result<SkillManifest, SdkError> {
        let url = format!("{}/api/v1/skills/{skill_id}", self.config.base_url);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(SdkError::Api {
                status: resp.status().as_u16(),
                message: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(resp.json().await?)
    }

    /// Installs a skill dynamically from Git, Markdown, or Archive.
    pub async fn install_skill(
        &self,
        input: &SkillInstallInput,
    ) -> Result<SkillManifest, SdkError> {
        let url = format!("{}/api/v1/skills/install", self.config.base_url);
        let resp = self.client.post(&url).json(input).send().await?;
        if !resp.status().is_success() {
            return Err(SdkError::Api {
                status: resp.status().as_u16(),
                message: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(resp.json().await?)
    }

    /// Uninstalls a user-installed skill.
    pub async fn delete_skill(&self, skill_id: &str) -> Result<(), SdkError> {
        let url = format!("{}/api/v1/skills/{skill_id}", self.config.base_url);
        let resp = self.client.delete(&url).send().await?;
        if !resp.status().is_success() {
            return Err(SdkError::Api {
                status: resp.status().as_u16(),
                message: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdk_config_trims_trailing_slash() {
        let cfg = SdkConfig::new("https://test.nla.run///");
        assert_eq!(cfg.base_url, "https://test.nla.run");
    }

    #[test]
    fn test_sdk_instantiation_with_bearer_token() {
        let cfg = SdkConfig::new("https://test.nla.run").with_token("test-key-123");
        let sdk = PulsarSdk::new(cfg);
        assert!(sdk.is_ok());
    }
}
