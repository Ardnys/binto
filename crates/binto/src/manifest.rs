use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value, value};

use crate::config::config_dir;
use crate::error::BintoError;

/// Declarative, portable list of tools binto should manage. Lives at
/// `~/.config/binto/manifest.toml` alongside `config.toml`. Unlike `state.toml` (a local
/// runtime cache of install paths / sha256 / etags), the manifest holds only the portable
/// identity of each tool — its `repo`, an optional pinned `tag`, and an optional install
/// `alias` — so it can be committed to dotfiles and replayed on another machine with
/// `binto sync`.
///
/// Reads parse into this typed view (comments are ignored, as they're not data). Writes go
/// through the format-preserving `*_and_save` methods, which edit the on-disk TOML document
/// in place so hand-written comments, ordering, and unrelated entries survive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub tools: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub repo: String,
    /// When set, the tool is pinned/locked to this exact release tag: `sync` installs it and
    /// `update` skips it. Absent means "track latest".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// When set, the binary is installed (and tracked) under this name instead of the
    /// repo-derived default, so `sync` reproduces the custom name. Absent means "use the
    /// repo name".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

impl Manifest {
    pub fn manifest_path() -> PathBuf {
        config_dir().join("manifest.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::manifest_path();

        if !path.exists() {
            return Ok(Manifest::default());
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        toml::from_str(&raw).map_err(|e| BintoError::StateCorrupted(e.to_string()).into())
    }

    pub fn get(&self, repo: &str) -> Option<&ManifestEntry> {
        self.tools.iter().find(|e| e.repo == repo)
    }

    /// The pinned tag for `repo`, if it is pinned.
    pub fn is_pinned(&self, repo: &str) -> Option<&str> {
        self.get(repo).and_then(|e| e.tag.as_deref())
    }

    pub fn iter(&self) -> impl Iterator<Item = &ManifestEntry> {
        self.tools.iter()
    }

    // ---- Format-preserving writes --------------------------------------------------------
    //
    // These load the file as a `toml_edit` document, mutate only the row they touch, and
    // write it back, so comments / blank lines / key order on every other line are kept
    // verbatim. The one thing they can't preserve is an inline comment on a key whose *value*
    // they're rewriting in the same operation — and even that is preserved for value updates
    // (see `set_str`), only dropped when the key itself is removed.

    /// Insert or update `repo`'s row, recording both `tag` and `alias` (`None` clears that
    /// key). Used by `binto install` / `binto sync`, which know both up front.
    pub fn record_and_save(repo: &str, tag: Option<&str>, alias: Option<&str>) -> Result<()> {
        // Hold the global lock across load→edit→write so concurrent `binto` processes don't lose
        // each other's manifest rows.
        let _guard = crate::lock::acquire()?;
        let mut doc = Self::load_doc()?;
        edit_tool(&mut doc, repo, |t| {
            set_str(t, "tag", tag);
            set_str(t, "alias", alias);
        })?;
        Self::write_doc(&doc)
    }

    /// Insert or update only `repo`'s pin `tag` (`None` clears it), leaving any `alias`
    /// untouched. Used by the `update --force` pin toggle and by `adopt`.
    pub fn set_tag_and_save(repo: &str, tag: Option<&str>) -> Result<()> {
        let _guard = crate::lock::acquire()?;
        let mut doc = Self::load_doc()?;
        edit_tool(&mut doc, repo, |t| set_str(t, "tag", tag))?;
        Self::write_doc(&doc)
    }

    /// Drop `repo`'s row. Returns whether a row was removed.
    pub fn remove_and_save(repo: &str) -> Result<bool> {
        let _guard = crate::lock::acquire()?;
        let mut doc = Self::load_doc()?;
        let removed = remove_row(&mut doc, repo)?;
        if removed {
            Self::write_doc(&doc)?;
        }
        Ok(removed)
    }

    /// Load the manifest as a format-preserving document (an empty document if absent).
    fn load_doc() -> Result<DocumentMut> {
        let path = Self::manifest_path();
        if !path.exists() {
            return Ok(DocumentMut::new());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        raw.parse::<DocumentMut>()
            .map_err(|e| BintoError::StateCorrupted(e.to_string()).into())
    }

    /// Atomically write `doc` back to the manifest path (write-temp-then-rename).
    fn write_doc(doc: &DocumentMut) -> Result<()> {
        let path = Self::manifest_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, doc.to_string())
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).context("failed to rename manifest file")?;
        Ok(())
    }
}

/// The `[[tools]]` array of tables, created empty if absent. Errors if `tools` exists but
/// isn't an array of tables (a hand-broken manifest).
fn tools_mut(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables> {
    let item = doc
        .as_table_mut()
        .entry("tools")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    item.as_array_of_tables_mut().ok_or_else(|| {
        anyhow::Error::from(BintoError::StateCorrupted(
            "`tools` is not an array of tables".to_string(),
        ))
    })
}

/// Index of the `[[tools]]` entry whose `repo` matches, if any.
fn find_index(aot: &ArrayOfTables, repo: &str) -> Option<usize> {
    aot.iter()
        .position(|t| t.get("repo").and_then(Item::as_str) == Some(repo))
}

/// Find `repo`'s table (inserting a fresh one with `repo` set if absent), then apply `f`.
fn edit_tool(doc: &mut DocumentMut, repo: &str, f: impl FnOnce(&mut Table)) -> Result<()> {
    let aot = tools_mut(doc)?;
    match find_index(aot, repo) {
        Some(idx) => f(aot.get_mut(idx).expect("index just found")),
        None => {
            let mut t = Table::new();
            t["repo"] = value(repo);
            f(&mut t);
            aot.push(t);
        }
    }
    Ok(())
}

/// Remove `repo`'s row without creating the array if it's missing. Returns whether it removed.
fn remove_row(doc: &mut DocumentMut, repo: &str) -> Result<bool> {
    let Some(item) = doc.as_table_mut().get_mut("tools") else {
        return Ok(false);
    };
    let Some(aot) = item.as_array_of_tables_mut() else {
        return Ok(false);
    };
    match find_index(aot, repo) {
        Some(idx) => {
            aot.remove(idx);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Set `key` to `val`, or remove it when `val` is `None`. Updating an existing value keeps
/// its surrounding decor (e.g. a trailing `# comment`); a fresh key gets default formatting.
fn set_str(table: &mut Table, key: &str, val: Option<&str>) {
    match val {
        Some(v) => {
            if let Some(existing) = table.get_mut(key).and_then(Item::as_value_mut) {
                let decor = existing.decor().clone();
                *existing = Value::from(v);
                *existing.decor_mut() = decor;
            } else {
                table[key] = value(v);
            }
        }
        None => {
            table.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(src: &str) -> DocumentMut {
        src.parse::<DocumentMut>().unwrap()
    }

    #[test]
    fn record_inserts_new_entry_with_alias() {
        let mut d = DocumentMut::new();
        edit_tool(&mut d, "BurntSushi/ripgrep", |t| {
            set_str(t, "tag", None);
            set_str(t, "alias", Some("rg"));
        })
        .unwrap();

        let out = d.to_string();
        assert!(out.contains("[[tools]]"));
        assert!(out.contains(r#"repo = "BurntSushi/ripgrep""#));
        assert!(out.contains(r#"alias = "rg""#));
        assert!(!out.contains("tag"));

        // Typed view sees it.
        let m: Manifest = toml::from_str(&out).unwrap();
        assert_eq!(
            m.get("BurntSushi/ripgrep").unwrap().alias.as_deref(),
            Some("rg")
        );
    }

    #[test]
    fn re_pinning_preserves_comments_and_other_entries() {
        let mut d = doc(r#"# my tools

[[tools]]
repo = "BurntSushi/ripgrep"

[[tools]]
repo = "sharkdp/bat"
tag = "v0.24.0"   # pinned: v0.25 broke theme
"#);

        edit_tool(&mut d, "sharkdp/bat", |t| {
            set_str(t, "tag", Some("v0.25.0"))
        })
        .unwrap();

        let out = d.to_string();
        // header comment, the untouched entry, and the inline comment all survive
        assert!(out.contains("# my tools"));
        assert!(out.contains("BurntSushi/ripgrep"));
        assert!(out.contains("# pinned: v0.25 broke theme"));
        // the value itself was updated
        assert!(out.contains("v0.25.0"));
        assert!(!out.contains("v0.24.0"));

        let m: Manifest = toml::from_str(&out).unwrap();
        assert_eq!(m.tools.len(), 2);
        assert_eq!(m.is_pinned("sharkdp/bat"), Some("v0.25.0"));
    }

    #[test]
    fn clearing_pin_keeps_alias() {
        let mut d = doc(r#"[[tools]]
repo = "BurntSushi/ripgrep"
tag = "14.1.0"
alias = "rg"
"#);

        edit_tool(&mut d, "BurntSushi/ripgrep", |t| set_str(t, "tag", None)).unwrap();

        let out = d.to_string();
        assert!(!out.contains("tag"));
        assert!(out.contains(r#"alias = "rg""#));
    }

    #[test]
    fn remove_keeps_commented_block_and_comments() {
        let mut d = doc(r#"[[tools]]
repo = "BurntSushi/ripgrep"

# kept for later
# [[tools]]
# repo = "junegunn/fzf"

[[tools]]
repo = "sharkdp/bat"
"#);

        assert!(remove_row(&mut d, "BurntSushi/ripgrep").unwrap());

        let out = d.to_string();
        assert!(!out.contains("BurntSushi/ripgrep"));
        assert!(out.contains("# kept for later"));
        assert!(out.contains("sharkdp/bat"));

        // The commented-out fzf block is invisible to the typed parse, so sync/prune never
        // sees it — exactly the "comment it out to disable" behaviour.
        let m: Manifest = toml::from_str(&out).unwrap();
        assert_eq!(m.tools.len(), 1);
        assert_eq!(m.tools[0].repo, "sharkdp/bat");
    }

    #[test]
    fn remove_absent_repo_reports_false() {
        let mut d = doc(r#"[[tools]]
repo = "a/b"
"#);
        assert!(!remove_row(&mut d, "missing/repo").unwrap());
    }

    #[test]
    fn is_pinned_reads_tag() {
        let m: Manifest = toml::from_str(
            r#"[[tools]]
repo = "a/b"

[[tools]]
repo = "c/d"
tag = "v2"
"#,
        )
        .unwrap();

        assert_eq!(m.is_pinned("a/b"), None);
        assert_eq!(m.is_pinned("c/d"), Some("v2"));
        assert_eq!(m.is_pinned("missing/repo"), None);
    }

    #[test]
    fn default_manifest_round_trips_empty() {
        let m = Manifest::default();
        let toml = toml::to_string_pretty(&m).unwrap();
        let back: Manifest = toml::from_str(&toml).unwrap();
        assert!(back.tools.is_empty());
    }
}
