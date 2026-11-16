use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigScope {
    Defaults,
    User,
    Project,
    Session,
    CliFlags,
}

impl ConfigScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Defaults => "defaults",
            Self::User => "user",
            Self::Project => "project",
            Self::Session => "session",
            Self::CliFlags => "cli_flags",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigLayer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_master_key: Option<String>,
    #[serde(flatten)]
    pub unknown_fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveSettings {
    pub bind_addr: String,
    pub default_provider: String,
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_model: Option<String>,
    pub theme: String,
    pub language: String,
    pub max_turns: u32,
    pub timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_master_key: Option<String>,
}

impl Default for EffectiveSettings {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8080".to_owned(),
            default_provider: "ollama".to_owned(),
            default_model: "llama3.2".to_owned(),
            fallback_model: None,
            theme: "dark".to_owned(),
            language: "en".to_owned(),
            max_turns: 25,
            timeout_seconds: 120,
            credential_master_key: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigDiagnosticSeverity {
    Warning,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDiagnostic {
    pub scope: ConfigScope,
    pub field: String,
    pub severity: ConfigDiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResolutionOutcome {
    pub settings: EffectiveSettings,
    pub provenance: BTreeMap<String, ConfigScope>,
    pub warnings: Vec<ConfigDiagnostic>,
    pub fatals: Vec<ConfigDiagnostic>,
}

impl ConfigResolutionOutcome {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.fatals.is_empty()
    }
}

const VALID_THEMES: &[&str] = &["dark", "light", "system", "high-contrast"];
const VALID_LANGUAGES: &[&str] = &["en", "es", "fr", "de", "pt", "zh", "ar"];

#[must_use]
pub fn resolve_settings_hierarchy(
    layers: &[(ConfigScope, ConfigLayer)],
) -> ConfigResolutionOutcome {
    let mut settings = EffectiveSettings::default();
    let mut provenance = BTreeMap::new();
    for key in [
        "bindAddr",
        "defaultProvider",
        "defaultModel",
        "fallbackModel",
        "theme",
        "language",
        "maxTurns",
        "timeoutSeconds",
    ] {
        provenance.insert(key.to_owned(), ConfigScope::Defaults);
    }

    let mut ordered: Vec<(ConfigScope, &ConfigLayer)> =
        layers.iter().map(|(s, l)| (*s, l)).collect();
    ordered.sort_by_key(|(scope, _)| *scope);

    let mut warnings = Vec::new();
    let mut fatals = Vec::new();

    for (scope, layer) in ordered {
        for unknown_key in layer.unknown_fields.keys() {
            warnings.push(ConfigDiagnostic {
                scope,
                field: unknown_key.clone(),
                severity: ConfigDiagnosticSeverity::Warning,
                message: format!(
                    "unrecognized setting `{unknown_key}` ignored; safe default retained"
                ),
            });
        }

        if let Some(addr) = &layer.bind_addr {
            let trimmed = addr.trim();
            if is_valid_host_port(trimmed) {
                settings.bind_addr = trimmed.to_owned();
                provenance.insert("bindAddr".to_owned(), scope);
            } else {
                fatals.push(ConfigDiagnostic {
                    scope,
                    field: "bindAddr".to_owned(),
                    severity: ConfigDiagnosticSeverity::Fatal,
                    message: format!("invalid network bind address `{trimmed}`"),
                });
            }
        }

        if let Some(key) = &layer.credential_master_key {
            if key.trim().len() < 16 {
                fatals.push(ConfigDiagnostic {
                    scope,
                    field: "credentialMasterKey".to_owned(),
                    severity: ConfigDiagnosticSeverity::Fatal,
                    message: "credentialMasterKey must be at least 16 characters".to_owned(),
                });
            } else {
                settings.credential_master_key = Some(key.trim().to_owned());
                provenance.insert("credentialMasterKey".to_owned(), scope);
            }
        }

        if let Some(prov) = &layer.default_provider
            && !prov.trim().is_empty()
        {
            settings.default_provider = prov.trim().to_owned();
            provenance.insert("defaultProvider".to_owned(), scope);
        }

        if let Some(model) = &layer.default_model
            && !model.trim().is_empty()
        {
            settings.default_model = model.trim().to_owned();
            provenance.insert("defaultModel".to_owned(), scope);
        }

        if let Some(fb) = &layer.fallback_model {
            let trimmed = fb.trim();
            settings.fallback_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            };
            provenance.insert("fallbackModel".to_owned(), scope);
        }

        if let Some(theme) = &layer.theme {
            let norm = theme.trim().to_ascii_lowercase();
            if VALID_THEMES.contains(&norm.as_str()) {
                settings.theme = norm;
                provenance.insert("theme".to_owned(), scope);
            } else {
                warnings.push(ConfigDiagnostic {
                    scope,
                    field: "theme".to_owned(),
                    severity: ConfigDiagnosticSeverity::Warning,
                    message: format!(
                        "unknown theme `{theme}`; falling back to `{}`",
                        settings.theme
                    ),
                });
            }
        }

        if let Some(lang) = &layer.language {
            let norm = lang.trim().to_ascii_lowercase();
            if VALID_LANGUAGES.contains(&norm.as_str()) {
                settings.language = norm;
                provenance.insert("language".to_owned(), scope);
            } else {
                warnings.push(ConfigDiagnostic {
                    scope,
                    field: "language".to_owned(),
                    severity: ConfigDiagnosticSeverity::Warning,
                    message: format!(
                        "unsupported language `{lang}`; falling back to `{}`",
                        settings.language
                    ),
                });
            }
        }

        if let Some(turns) = layer.max_turns {
            if (1..=200).contains(&turns) {
                settings.max_turns = turns;
                provenance.insert("maxTurns".to_owned(), scope);
            } else {
                warnings.push(ConfigDiagnostic {
                    scope,
                    field: "maxTurns".to_owned(),
                    severity: ConfigDiagnosticSeverity::Warning,
                    message: format!(
                        "maxTurns `{turns}` out of bounds (1..=200); retaining `{}`",
                        settings.max_turns
                    ),
                });
            }
        }

        if let Some(timeout) = layer.timeout_seconds {
            if (5..=3600).contains(&timeout) {
                settings.timeout_seconds = timeout;
                provenance.insert("timeoutSeconds".to_owned(), scope);
            } else {
                warnings.push(ConfigDiagnostic {
                    scope,
                    field: "timeoutSeconds".to_owned(),
                    severity: ConfigDiagnosticSeverity::Warning,
                    message: format!(
                        "timeoutSeconds `{timeout}` out of bounds (5..=3600); retaining `{}`",
                        settings.timeout_seconds
                    ),
                });
            }
        }
    }

    ConfigResolutionOutcome {
        settings,
        provenance,
        warnings,
        fatals,
    }
}

#[must_use]
pub fn serialize_minimal_diff(custom: &EffectiveSettings, defaults: &EffectiveSettings) -> Value {
    let mut map = serde_json::Map::new();
    if custom.bind_addr != defaults.bind_addr {
        map.insert("bindAddr".to_owned(), json!(custom.bind_addr));
    }
    if custom.default_provider != defaults.default_provider {
        map.insert("defaultProvider".to_owned(), json!(custom.default_provider));
    }
    if custom.default_model != defaults.default_model {
        map.insert("defaultModel".to_owned(), json!(custom.default_model));
    }
    if custom.fallback_model != defaults.fallback_model {
        map.insert("fallbackModel".to_owned(), json!(custom.fallback_model));
    }
    if custom.theme != defaults.theme {
        map.insert("theme".to_owned(), json!(custom.theme));
    }
    if custom.language != defaults.language {
        map.insert("language".to_owned(), json!(custom.language));
    }
    if custom.max_turns != defaults.max_turns {
        map.insert("maxTurns".to_owned(), json!(custom.max_turns));
    }
    if custom.timeout_seconds != defaults.timeout_seconds {
        map.insert("timeoutSeconds".to_owned(), json!(custom.timeout_seconds));
    }
    Value::Object(map)
}

fn is_valid_host_port(addr: &str) -> bool {
    let Some((host, port_str)) = addr.rsplit_once(':') else {
        return false;
    };
    if host.trim().is_empty() || host.contains(char::is_whitespace) {
        return false;
    }
    port_str.parse::<u16>().is_ok()
}
