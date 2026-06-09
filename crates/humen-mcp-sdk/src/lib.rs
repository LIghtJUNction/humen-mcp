//! Shared types for `humen-mcp` community plugin manifests.
//!
//! A plugin is a declarative manifest. The server loads manifests from a plugin
//! directory and exposes their request templates, route strategies, scoring
//! rules, and third-party channels through MCP tools.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const MANIFEST_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumenTaskKind {
    Choice,
    Judgment,
    Text,
    ImageReview,
    Steps,
}

impl Default for HumenTaskKind {
    fn default() -> Self {
        Self::Text
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequestTemplate {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub kind: HumenTaskKind,
    #[serde(default)]
    pub prompt_template: String,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteStrategy {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub prefer_online: bool,
    #[serde(default)]
    pub minimum_reputation: Option<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ScoringRule {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub weight: f64,
    #[serde(default)]
    pub rubric: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThirdPartyChannel {
    pub id: String,
    pub title: String,
    pub description: String,
    pub kind: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub config_schema: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HumenPluginManifest {
    #[serde(default = "manifest_schema_version")]
    pub schema_version: u16,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub request_templates: Vec<RequestTemplate>,
    #[serde(default)]
    pub route_strategies: Vec<RouteStrategy>,
    #[serde(default)]
    pub scoring_rules: Vec<ScoringRule>,
    #[serde(default)]
    pub channels: Vec<ThirdPartyChannel>,
}

pub fn manifest_schema_version() -> u16 {
    MANIFEST_SCHEMA_VERSION
}

impl HumenPluginManifest {
    pub fn validate(&self) -> Result<(), String> {
        validate_id("plugin id", &self.id)?;
        validate_required("plugin name", &self.name)?;
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(format!(
                "unsupported schema_version {}; expected {}",
                self.schema_version, MANIFEST_SCHEMA_VERSION
            ));
        }
        for template in &self.request_templates {
            validate_id("request template id", &template.id)?;
            validate_required("request template title", &template.title)?;
        }
        for strategy in &self.route_strategies {
            validate_id("route strategy id", &strategy.id)?;
            validate_required("route strategy title", &strategy.title)?;
            if let Some(minimum_reputation) = strategy.minimum_reputation {
                if minimum_reputation > 10 {
                    return Err(format!(
                        "route strategy {} minimum_reputation must be 0..10",
                        strategy.id
                    ));
                }
            }
        }
        for rule in &self.scoring_rules {
            validate_id("scoring rule id", &rule.id)?;
            validate_required("scoring rule title", &rule.title)?;
        }
        for channel in &self.channels {
            validate_id("channel id", &channel.id)?;
            validate_required("channel title", &channel.title)?;
            validate_id("channel kind", &channel.kind)?;
        }
        Ok(())
    }
}

fn validate_required(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} is required"));
    }
    Ok(())
}

fn validate_id(label: &str, value: &str) -> Result<(), String> {
    validate_required(label, value)?;
    let trimmed = value.trim();
    if trimmed != value {
        return Err(format!("{label} must not have surrounding whitespace"));
    }
    if trimmed.len() > 96 {
        return Err(format!("{label} is too long"));
    }
    if trimmed
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
    {
        Ok(())
    } else {
        Err(format!(
            "{label} must use lowercase ASCII letters, numbers, '_' or '-'"
        ))
    }
}
