use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::GhrError;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct State {
    #[serde(default)]
    pub tools: IndexMap<String, ToolEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolEntry {
    pub repo: String,
    pub installed_tag: String,
    pub install_path: PathBuf,
    pub binary_name: String,
    pub asset_pattern: String,
    pub installed_sha256: Option<String>,
    pub etag: Option<String>,
    pub last_checked: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
}

impl State {
    pub fn state_path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"))
            .join("ghr/state.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::state_path();

        if !path.exists() {
            return Ok(State::default());
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        toml::from_str(&raw).map_err(|e| GhrError::StateCorrupted(e.to_string()).into())
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let raw = toml::to_string_pretty(self).context("failed to serialize state")?;

        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, raw).with_context(|| format!("failed to write {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| "failed to rename state file".to_string())?;

        Ok(())
    }
}
