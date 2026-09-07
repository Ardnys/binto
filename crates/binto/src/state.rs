use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::BintoError;

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
    /// Which binary of `repo` this entry is, as read off the asset name it was installed
    /// from: `tool-server` for `tool-server-1.2.3-x86_64-unknown-linux-musl.tar.xz`.
    ///
    /// Distinct from `binary_name`, which is the filename on disk and may be an `--alias`.
    /// A repo shipping several binaries needs both — the alias says what the user calls it,
    /// the stem says which one it is.
    ///
    /// Empty for entries written before this was recorded, and for adopted binaries, which
    /// have no asset to read. Both fill themselves in on the next install or update.
    #[serde(default)]
    pub stem: String,
    pub asset_pattern: String,
    pub installed_sha256: Option<String>,
    pub etag: Option<String>,
    pub last_checked: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
}

impl ToolEntry {
    /// Whether a release published at `latest_published` is newer than what's installed.
    /// A missing local timestamp (e.g. freshly adopted) is treated as "always behind".
    pub fn is_behind(&self, latest_published: DateTime<Utc>) -> bool {
        self.published_at
            .map(|installed| latest_published > installed)
            .unwrap_or(true)
    }

    /// Directory the binary should be (re)installed into: the parent of the current
    /// install path, falling back to `fallback` for entries without a usable parent.
    /// Keeps adopted tools (which may live outside the default dir) in place.
    pub fn install_dir<'a>(&'a self, fallback: &'a Path) -> &'a Path {
        self.install_path.parent().unwrap_or(fallback)
    }

    /// Builder-style override of the cached ETag, used after a successful install so the
    /// next conditional request can short-circuit with `304 Not Modified`.
    pub fn with_etag(mut self, etag: Option<String>) -> Self {
        self.etag = etag;
        self
    }
}

impl State {
    pub fn state_path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"))
            .join("binto/state.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::state_path();

        if !path.exists() {
            return Ok(State::default());
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        toml::from_str(&raw).map_err(|e| BintoError::StateCorrupted(e.to_string()).into())
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

    /// Apply `f` to the freshest on-disk state under the global cross-process lock, then persist.
    /// Re-reading inside the lock is what prevents lost updates: concurrent `binto` processes
    /// serialize *just* this short critical section, so a parallel install can't overwrite a
    /// sibling's entry. Slow work (downloads, extraction) must happen *before* this call, not
    /// inside `f`.
    pub fn mutate<R>(f: impl FnOnce(&mut State) -> R) -> Result<R> {
        let _guard = crate::lock::acquire()?;
        let mut state = State::load()?;
        let out = f(&mut state);
        state.save()?;
        Ok(out)
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&ToolEntry> {
        self.tools.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Whether any managed tool was installed from `repo`. Used by `sync` to skip
    /// manifest entries that are already installed (state is keyed by binary name,
    /// the manifest by repo, so the lookup is by value here).
    pub fn contains_repo(&self, repo: &str) -> bool {
        self.tools.values().any(|e| e.repo == repo)
    }

    /// Look up a tool, returning a typed `UnknownTool` error if it isn't managed.
    pub fn require(&self, name: &str) -> Result<&ToolEntry> {
        self.tools.get(name).ok_or_else(|| {
            BintoError::UnknownTool {
                name: name.to_string(),
            }
            .into()
        })
    }

    /// Insert or replace an entry, always keyed by its own `binary_name` so the map key
    /// and the entry can never drift apart.
    pub fn upsert(&mut self, entry: ToolEntry) {
        self.tools.insert(entry.binary_name.clone(), entry);
    }

    /// Stamp a tool's `last_checked` to now. No-op if the tool isn't present.
    pub fn touch_checked(&mut self, name: &str) {
        if let Some(e) = self.tools.get_mut(name) {
            e.last_checked = Some(Utc::now());
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<ToolEntry> {
        self.tools.shift_remove(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ToolEntry)> {
        self.tools.iter().map(|(k, v)| (k.as_str(), v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, published: Option<DateTime<Utc>>) -> ToolEntry {
        ToolEntry {
            repo: format!("owner/{name}"),
            installed_tag: "v1.0.0".to_string(),
            install_path: PathBuf::from(format!("/home/u/.local/bin/{name}")),
            binary_name: name.to_string(),
            stem: name.to_string(),
            asset_pattern: String::new(),
            installed_sha256: None,
            etag: None,
            last_checked: None,
            published_at: published,
        }
    }

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn is_behind_true_when_release_is_newer() {
        let e = entry("bat", Some(ts("2024-01-01T00:00:00Z")));
        assert!(e.is_behind(ts("2024-06-01T00:00:00Z")));
    }

    #[test]
    fn is_behind_false_when_release_is_same_or_older() {
        let e = entry("bat", Some(ts("2024-06-01T00:00:00Z")));
        assert!(!e.is_behind(ts("2024-06-01T00:00:00Z")));
        assert!(!e.is_behind(ts("2024-01-01T00:00:00Z")));
    }

    #[test]
    fn is_behind_true_when_no_local_timestamp() {
        let e = entry("bat", None);
        assert!(e.is_behind(ts("2000-01-01T00:00:00Z")));
    }

    #[test]
    fn install_dir_uses_parent_then_fallback() {
        let e = entry("bat", None);
        let fallback = Path::new("/fallback");
        assert_eq!(e.install_dir(fallback), Path::new("/home/u/.local/bin"));

        let mut rootless = entry("bat", None);
        rootless.install_path = PathBuf::from("bat");
        // parent of a bare filename is "" — still Some, not the fallback
        assert_eq!(rootless.install_dir(fallback), Path::new(""));
    }

    #[test]
    fn require_errors_for_unknown_tool() {
        let state = State::default();
        let err = state.require("nope").unwrap_err();
        assert!(matches!(
            err.downcast_ref::<BintoError>(),
            Some(BintoError::UnknownTool { .. })
        ));
    }

    #[test]
    fn upsert_keys_by_binary_name_and_replaces() {
        let mut state = State::default();
        state.upsert(entry("bat", None));
        assert!(state.contains("bat"));

        let updated = entry("bat", None).with_etag(Some("abc".to_string()));
        state.upsert(updated);
        assert_eq!(state.tools.len(), 1);
        assert_eq!(state.get("bat").unwrap().etag.as_deref(), Some("abc"));
    }

    /// Every `state.toml` written before stems existed lacks the key. Loading one must not
    /// fail — the entry simply does not know which binary of its repo it is until the next
    /// install or update rebuilds it.
    #[test]
    fn a_state_file_without_stems_still_loads() {
        let raw = r#"
[tools.bat]
repo = "sharkdp/bat"
installed_tag = "v0.24.0"
install_path = "/home/u/.local/bin/bat"
binary_name = "bat"
asset_pattern = "bat-*-x86_64-unknown-linux-gnu.tar.gz"
"#;
        let state: State = toml::from_str(raw).unwrap();
        let bat = state.get("bat").unwrap();
        assert_eq!(bat.binary_name, "bat");
        assert_eq!(bat.stem, "");
    }

    /// The stem is the variant, `binary_name` is the filename — an alias moves one and
    /// leaves the other alone, which is what lets a repo ship several binaries.
    #[test]
    fn an_alias_renames_the_binary_without_changing_which_one_it_is() {
        let mut aliased = entry("rg", None);
        aliased.repo = "BurntSushi/ripgrep".to_string();
        aliased.stem = "ripgrep".to_string();

        let mut state = State::default();
        state.upsert(aliased);

        // Keyed by the name the user chose...
        assert!(state.contains("rg"));
        // ...while still recording which binary of the repo it actually is.
        assert_eq!(state.get("rg").unwrap().stem, "ripgrep");
    }

    #[test]
    fn touch_checked_sets_timestamp_and_ignores_missing() {
        let mut state = State::default();
        state.upsert(entry("bat", None));
        assert!(state.get("bat").unwrap().last_checked.is_none());
        state.touch_checked("bat");
        assert!(state.get("bat").unwrap().last_checked.is_some());
        // no panic for an absent tool
        state.touch_checked("ghost");
    }
}
