use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub install_dir: PathBuf,
    pub github_token: Option<String>,
    pub include_prereleases: bool,
    pub check_interval_hours: u32,
    pub notify: NotifyMode,
}

// TODO: notifications are not implemented
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotifyMode {
    #[default]
    Terminal,
    Desktop,
    None,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            install_dir: default_install_dir(),
            github_token: None,
            include_prereleases: false,
            check_interval_hours: 24,
            notify: NotifyMode::Terminal,
        }
    }
}

fn default_install_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".local/bin")
}

impl Config {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
            .join("ghr/config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();

        if !path.exists() {
            let config = Config::default();
            config.save()?;
            return Ok(config);
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        let mut config: Config =
            toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;

        // Expand ~ in install_dir
        if let Ok(stripped) = config.install_dir.strip_prefix("~")
            && let Some(home) = dirs::home_dir()
        {
            config.install_dir = home.join(stripped);
        }

        // Empty string token → None
        if config.github_token.as_deref() == Some("") {
            config.github_token = None;
        }

        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        // Store install_dir with ~ for readability
        let mut display = self.clone();
        if let Some(home) = dirs::home_dir()
            && let Ok(rel) = display.install_dir.strip_prefix(&home)
        {
            display.install_dir = PathBuf::from("~").join(rel);
        }

        let raw = toml::to_string_pretty(&display).context("failed to serialize config")?;
        std::fs::write(&path, raw)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }
}
