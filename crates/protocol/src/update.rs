//! Over-The-Air (OTA) update manifests, release channel descriptors, and signature validation schemas.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Supported release distribution channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Stable,
    Nightly,
}

impl UpdateChannel {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        }
    }
}

impl fmt::Display for UpdateChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Downloadable asset for a specific target platform/architecture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformAsset {
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_ed25519: Option<String>,
}

/// Standalone OTA update bundle or delta package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageEntry {
    pub version: String,
    pub build: u64,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub min_runtime_version: String,
}

/// Cryptographically verifiable release manifest describing available updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateManifest {
    pub id: String,
    pub version: String,
    pub channel: UpdateChannel,
    pub notes: String,
    pub prerelease: bool,
    pub platforms: HashMap<String, PlatformAsset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<PackageEntry>,
    pub published_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateValidationError {
    EmptyAppId,
    EmptyVersion,
    InvalidSha256Hex { asset: String },
    NoPlatformsOrPackage,
}

impl fmt::Display for UpdateValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyAppId => write!(f, "update manifest id must not be empty"),
            Self::EmptyVersion => write!(f, "update manifest version must not be empty"),
            Self::InvalidSha256Hex { asset } => {
                write!(f, "invalid sha256 digest for asset '{asset}'")
            }
            Self::NoPlatformsOrPackage => {
                write!(
                    f,
                    "update manifest must declare at least one platform or package"
                )
            }
        }
    }
}

impl std::error::Error for UpdateValidationError {}

impl UpdateManifest {
    /// Validates structural integrity and hex digest formats.
    pub fn validate(&self) -> Result<(), UpdateValidationError> {
        if self.id.trim().is_empty() {
            return Err(UpdateValidationError::EmptyAppId);
        }
        if self.version.trim().is_empty() {
            return Err(UpdateValidationError::EmptyVersion);
        }
        if self.platforms.is_empty() && self.package.is_none() {
            return Err(UpdateValidationError::NoPlatformsOrPackage);
        }

        for (platform, asset) in &self.platforms {
            if !crate::artifact::is_valid_sha256_hex(&asset.sha256) {
                return Err(UpdateValidationError::InvalidSha256Hex {
                    asset: platform.clone(),
                });
            }
        }

        if let Some(pkg) = &self.package
            && !crate::artifact::is_valid_sha256_hex(&pkg.sha256)
        {
            return Err(UpdateValidationError::InvalidSha256Hex {
                asset: "package".into(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_update_manifest() {
        let mut platforms = HashMap::new();
        platforms.insert(
            "macos-aarch64".into(),
            PlatformAsset {
                url: "https://nla.run/dist/Natuleadan-macos-arm64.dmg".into(),
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
                size_bytes: 1234567,
                signature_ed25519: Some("sig_mock_b64".into()),
            },
        );

        let manifest = UpdateManifest {
            id: "dev.natuleadan.pulsar".into(),
            version: "0.2.0".into(),
            channel: UpdateChannel::Stable,
            notes: "Stable security release".into(),
            prerelease: false,
            platforms,
            package: None,
            published_at_ms: 1700000000000,
        };

        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn test_invalid_sha256_rejected() {
        let mut platforms = HashMap::new();
        platforms.insert(
            "linux-x86_64".into(),
            PlatformAsset {
                url: "https://nla.run/dist/natuleadan.tar.gz".into(),
                sha256: "not-a-valid-hex".into(),
                size_bytes: 1000,
                signature_ed25519: None,
            },
        );

        let manifest = UpdateManifest {
            id: "dev.natuleadan.pulsar".into(),
            version: "0.2.0".into(),
            channel: UpdateChannel::Nightly,
            notes: "Nightly build".into(),
            prerelease: true,
            platforms,
            package: None,
            published_at_ms: 1700000000000,
        };

        assert!(matches!(
            manifest.validate(),
            Err(UpdateValidationError::InvalidSha256Hex { .. })
        ));
    }
}
