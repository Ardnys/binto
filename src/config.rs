use std::path::{Path, PathBuf};

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
    pub prefer_libc: Libc,
}

/// Which libc flavor to prefer when a release ships both. The non-preferred flavor stays
/// eligible (a tool shipping only the other still installs) — this only shifts the soft score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Libc {
    #[default]
    Gnu,
    Musl,
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
            prefer_libc: Libc::Gnu,
        }
    }
}

// TODO: .local/share/binto/bin is a safer default than populating .local/bin, which could be used already
fn default_install_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"))
        .join("binto/bin")
}

/// Expand a leading `~` in `path` to the user's home directory. Paths without a `~` prefix
/// (and the case where the home dir can't be resolved) are returned unchanged.
/// TODO: idk if this is enough. there's a tilde crate for this
pub fn expand_tilde(path: &Path) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~")
        && let Some(home) = dirs::home_dir()
    {
        home.join(stripped)
    } else {
        path.to_path_buf()
    }
}

/// The binto config directory (`~/.config/binto`). Shared by `config.toml` and `manifest.toml`.
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
        .join("binto")
}

impl Config {
    pub fn config_path() -> PathBuf {
        config_dir().join("config.toml")
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
        config.install_dir = expand_tilde(&config.install_dir);

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
