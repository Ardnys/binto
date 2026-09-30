use std::path::PathBuf;

use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

use crate::config::config_dir;
use crate::error::BintoError;

/// The tag value meaning "track whatever the newest release is" rather than pinning.
const LATEST: &str = "latest";

/// Declarative, portable list of tools binto should manage. Lives at
/// `~/.config/binto/manifest.toml` alongside `config.toml`. Unlike `state.toml` (a local
/// runtime cache of install paths / sha256 / etags), the manifest holds only the portable
/// identity of each tool — the repo, which of its binaries, an optional pinned tag, and an
/// optional install alias — so it can be committed to dotfiles and replayed on another
/// machine with `binto sync`.
///
/// The repo is the key, and its value is either a tag string or a table of options — the
/// same shape Cargo uses for dependencies. A repo shipping several binaries lists them
/// under `binaries`, where the rule repeats one level down:
///
/// ```toml
/// [tools]
/// "sharkdp/bat" = "latest"
/// "cli/cli" = "v2.45.0"
/// "BurntSushi/ripgrep" = { alias = "rg" }
///
/// [tools."restatedev/restate".binaries]
/// restate-cli = "latest"
/// restate-server = { tag = "v1.1.0", alias = "restated" }
/// ```
///
/// When `binaries` is present the repo entry is purely a container: every setting lives per
/// binary, and a repo-level `tag`/`alias` is not read. That keeps "does the repo tag
/// override the binary tag?" from ever being a question.
///
/// Reads parse into this typed view (comments are ignored, as they're not data). Writes go
/// through the format-preserving `*_and_save` methods, which edit the on-disk TOML document
/// in place so hand-written comments, ordering, and unrelated entries survive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    tools: IndexMap<String, RepoSpec>,
}

/// What the manifest says about one repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum RepoSpec {
    /// A bare tag: `"latest"` to track the newest release, or a tag to pin to.
    Tag(String),
    Table(RepoTable),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct RepoTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    /// Present only when the repo ships several binaries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    binaries: Option<IndexMap<String, BinarySpec>>,
}

/// What the manifest says about one binary of a multi-binary repo. Same rule as
/// [`RepoSpec`], minus the nesting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum BinarySpec {
    Tag(String),
    Table(BinaryTable),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct BinaryTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
}

/// One installable thing the manifest names, flattened out of the nested file shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub repo: String,
    /// Which binary of `repo`. `None` when the manifest names no binary, which is every
    /// repo shipping just one.
    pub binary: Option<String>,
    /// `None` means "track the latest release"; `Some` pins.
    pub tag: Option<String>,
    pub alias: Option<String>,
}

/// A written tag of `latest` is how the file spells "not pinned".
fn tag_of(raw: &str) -> Option<String> {
    (raw != LATEST).then(|| raw.to_string())
}

impl RepoSpec {
    /// The `(tag, alias)` this repo states at its own level. Meaningless when `binaries`
    /// is present, and not consulted then.
    fn own(&self) -> (Option<String>, Option<String>) {
        match self {
            RepoSpec::Tag(raw) => (tag_of(raw), None),
            RepoSpec::Table(t) => (t.tag.as_deref().and_then(tag_of), t.alias.clone()),
        }
    }

    fn binaries(&self) -> Option<&IndexMap<String, BinarySpec>> {
        match self {
            RepoSpec::Tag(_) => None,
            RepoSpec::Table(t) => t.binaries.as_ref(),
        }
    }
}

impl BinarySpec {
    fn own(&self) -> (Option<String>, Option<String>) {
        match self {
            BinarySpec::Tag(raw) => (tag_of(raw), None),
            BinarySpec::Table(t) => (t.tag.as_deref().and_then(tag_of), t.alias.clone()),
        }
    }
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

        toml::from_str(&raw).map_err(|e| BintoError::ManifestCorrupted(e.to_string()).into())
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Every binary the manifest asks for, one entry each. A repo listing no `binaries`
    /// yields a single entry with `binary: None`.
    pub fn iter(&self) -> impl Iterator<Item = ManifestEntry> + '_ {
        self.tools
            .iter()
            .flat_map(|(repo, spec)| match spec.binaries() {
                Some(binaries) => binaries
                    .iter()
                    .map(|(name, bin)| {
                        let (tag, alias) = bin.own();
                        ManifestEntry {
                            repo: repo.clone(),
                            binary: Some(name.clone()),
                            tag,
                            alias,
                        }
                    })
                    .collect::<Vec<_>>(),
                None => {
                    let (tag, alias) = spec.own();
                    vec![ManifestEntry {
                        repo: repo.clone(),
                        binary: None,
                        tag,
                        alias,
                    }]
                }
            })
    }

    /// The entry for one binary of `repo`, or for the repo itself when `binary` is `None`.
    pub fn get(&self, repo: &str, binary: Option<&str>) -> Option<ManifestEntry> {
        let spec = self.tools.get(repo)?;
        match (spec.binaries(), binary) {
            // A repo listing binaries only answers for the ones it lists.
            (Some(binaries), Some(name)) => {
                let (tag, alias) = binaries.get(name)?.own();
                Some(ManifestEntry {
                    repo: repo.to_string(),
                    binary: Some(name.to_string()),
                    tag,
                    alias,
                })
            }
            (Some(_), None) => None,
            (None, _) => {
                let (tag, alias) = spec.own();
                Some(ManifestEntry {
                    repo: repo.to_string(),
                    binary: None,
                    tag,
                    alias,
                })
            }
        }
    }

    /// The tag this binary is pinned to, if it is pinned.
    ///
    /// A pin is per binary: pinning `tool-cli` must not freeze `tool-server` alongside it.
    pub fn is_pinned(&self, repo: &str, binary: Option<&str>) -> Option<String> {
        self.get(repo, binary).and_then(|e| e.tag)
    }

    /// Whether the manifest still asks for an installed tool, given the binary it turned
    /// out to be. Used by `sync --prune`, where state is keyed by install name but the
    /// manifest is keyed by repo.
    ///
    /// A `stem` that is empty (nothing to read off the asset name) matches any repo the
    /// manifest names without enumerating binaries.
    pub fn covers(&self, repo: &str, stem: &str) -> bool {
        let Some(spec) = self.tools.get(repo) else {
            return false;
        };
        match spec.binaries() {
            Some(binaries) => binaries.contains_key(stem),
            None => true,
        }
    }

    // ---- Format-preserving writes --------------------------------------------------------
    //
    // These load the file as a `toml_edit` document, mutate only the entry they touch, and
    // write it back, so comments / blank lines / key order on every other line are kept
    // verbatim. A value being rewritten keeps its surrounding decor, including a trailing
    // `# comment`; only a key being removed outright loses it.

    /// Insert or update one binary's entry, recording both `tag` and `alias`.
    /// Used by `binto install`, which knows both up front.
    pub fn record_and_save(
        repo: &str,
        binary: Option<&str>,
        tag: Option<&str>,
        alias: Option<&str>,
    ) -> Result<()> {
        // Hold the global lock across load→edit→write so concurrent `binto` processes don't lose
        // each other's manifest entries.
        let _guard = crate::lock::acquire()?;
        let mut doc = Self::load_doc()?;
        set_spec(&mut doc, repo, binary, spec_value(tag, alias))?;
        Self::write_doc(&doc)
    }

    /// Insert or update only the pin, leaving any `alias` untouched. Used by the
    /// `update --force` pin toggle and by `adopt`.
    pub fn set_tag_and_save(repo: &str, binary: Option<&str>, tag: Option<&str>) -> Result<()> {
        let _guard = crate::lock::acquire()?;
        let mut doc = Self::load_doc()?;
        let alias = existing_alias(&doc, repo, binary);
        set_spec(&mut doc, repo, binary, spec_value(tag, alias.as_deref()))?;
        Self::write_doc(&doc)
    }

    /// Drop one binary's entry, or the whole repo when `binary` is `None`. Removing the
    /// last listed binary removes the repo with it, so no empty container is left behind.
    /// Returns whether anything was removed.
    pub fn remove_and_save(repo: &str, binary: Option<&str>) -> Result<bool> {
        let _guard = crate::lock::acquire()?;
        let mut doc = Self::load_doc()?;
        let removed = remove_spec(&mut doc, repo, binary);
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

/// The `[tools]` table, created empty if absent. Errors if `tools` exists but is not a
/// table (a hand-broken manifest).
fn tools_mut(doc: &mut DocumentMut) -> Result<&mut Table> {
    let item = doc
        .as_table_mut()
        .entry("tools")
        .or_insert(Item::Table(Table::new()));
    item.as_table_mut().ok_or_else(|| {
        anyhow::Error::from(BintoError::StateCorrupted(
            "`tools` is not a table".to_string(),
        ))
    })
}

/// The value a spec is written as: a bare tag string when that is all there is, and an
/// inline table once an alias has to ride along.
fn spec_value(tag: Option<&str>, alias: Option<&str>) -> Value {
    match alias {
        None => Value::from(tag.unwrap_or(LATEST)),
        Some(alias) => {
            let mut table = InlineTable::new();
            if let Some(tag) = tag {
                table.insert("tag", Value::from(tag));
            }
            table.insert("alias", Value::from(alias));
            Value::InlineTable(table)
        }
    }
}

/// Replace `key`'s value, keeping the decor (a trailing `# comment`) an existing one had.
fn set_value(table: &mut Table, key: &str, value: Value) {
    if let Some(existing) = table.get_mut(key).and_then(Item::as_value_mut) {
        let decor = existing.decor().clone();
        *existing = value;
        *existing.decor_mut() = decor;
    } else {
        table.insert(key, Item::Value(value));
    }
}

/// Write `value` at `tools.<repo>`, or at `tools.<repo>.binaries.<binary>`.
///
/// Naming a binary on a repo previously written as a bare tag converts it to a container.
/// The repo-level tag is dropped in that conversion: once binaries are enumerated it would
/// never be read again, and leaving it would be a key that silently does nothing.
fn set_spec(doc: &mut DocumentMut, repo: &str, binary: Option<&str>, value: Value) -> Result<()> {
    let tools = tools_mut(doc)?;

    let Some(binary) = binary else {
        set_value(tools, repo, value);
        return Ok(());
    };

    let repo_item = tools.entry(repo).or_insert(Item::Table(Table::new()));
    if repo_item.as_table().is_none() {
        *repo_item = Item::Table(Table::new());
    }
    let repo_table = repo_item.as_table_mut().expect("just made it a table");

    let binaries_item = repo_table
        .entry("binaries")
        .or_insert(Item::Table(Table::new()));
    if binaries_item.as_table().is_none() {
        *binaries_item = Item::Table(Table::new());
    }
    let binaries = binaries_item.as_table_mut().expect("just made it a table");

    set_value(binaries, binary, value);
    Ok(())
}

/// The alias already recorded for this entry, so a tag-only write does not drop it.
fn existing_alias(doc: &DocumentMut, repo: &str, binary: Option<&str>) -> Option<String> {
    let entry = doc.get("tools")?.get(repo)?;
    let entry = match binary {
        Some(name) => entry.get("binaries")?.get(name)?,
        None => entry,
    };
    entry
        .get("alias")
        .and_then(Item::as_str)
        .map(str::to_string)
}

/// Remove an entry without creating anything that was missing. Returns whether it removed.
fn remove_spec(doc: &mut DocumentMut, repo: &str, binary: Option<&str>) -> bool {
    let Some(tools) = doc.get_mut("tools").and_then(Item::as_table_mut) else {
        return false;
    };

    let Some(binary) = binary else {
        return tools.remove(repo).is_some();
    };

    let Some(binaries) = tools
        .get_mut(repo)
        .and_then(|repo| repo.get_mut("binaries"))
        .and_then(Item::as_table_mut)
    else {
        return false;
    };

    if binaries.remove(binary).is_none() {
        return false;
    }
    // Removing the last binary leaves a container asking for nothing; drop the repo too.
    if binaries.is_empty() {
        tools.remove(repo);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(src: &str) -> DocumentMut {
        src.parse::<DocumentMut>().unwrap()
    }

    fn parse(src: &str) -> Manifest {
        toml::from_str(src).unwrap()
    }

    #[test]
    fn a_bare_tag_string_is_the_whole_entry() {
        let m = parse(
            r#"
[tools]
"sharkdp/bat" = "latest"
"cli/cli" = "v2.45.0"
"#,
        );

        let bat = m.get("sharkdp/bat", None).unwrap();
        assert_eq!(
            bat.tag, None,
            "`latest` means unpinned, not a tag named latest"
        );
        assert_eq!(bat.alias, None);

        assert_eq!(m.is_pinned("cli/cli", None).as_deref(), Some("v2.45.0"));
    }

    #[test]
    fn a_table_carries_an_alias_alongside_the_tag() {
        let m = parse(
            r#"
[tools]
"BurntSushi/ripgrep" = { alias = "rg" }
"junegunn/fzf" = { tag = "0.44.0", alias = "f" }
"#,
        );

        let rg = m.get("BurntSushi/ripgrep", None).unwrap();
        assert_eq!(rg.alias.as_deref(), Some("rg"));
        assert_eq!(rg.tag, None);

        let fzf = m.get("junegunn/fzf", None).unwrap();
        assert_eq!(fzf.tag.as_deref(), Some("0.44.0"));
        assert_eq!(fzf.alias.as_deref(), Some("f"));
    }

    #[test]
    fn a_repo_shipping_several_binaries_lists_them_each() {
        let m = parse(
            r#"
[tools."restatedev/restate".binaries]
restate-cli = "latest"
restate-server = { tag = "v1.1.0", alias = "restated" }
"#,
        );

        let entries: Vec<ManifestEntry> = m.iter().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].binary.as_deref(), Some("restate-cli"));
        assert_eq!(entries[1].alias.as_deref(), Some("restated"));
    }

    /// The coupling this whole change exists to remove: pinning one binary must leave its
    /// siblings tracking latest.
    #[test]
    fn a_pin_applies_to_one_binary_not_the_whole_repo() {
        let m = parse(
            r#"
[tools."restatedev/restate".binaries]
restate-cli = "latest"
restate-server = "v1.1.0"
"#,
        );

        assert_eq!(
            m.is_pinned("restatedev/restate", Some("restate-server"))
                .as_deref(),
            Some("v1.1.0")
        );
        assert_eq!(m.is_pinned("restatedev/restate", Some("restate-cli")), None);
    }

    #[test]
    fn a_repo_listing_binaries_answers_only_for_those() {
        let m = parse(
            r#"
[tools."acme/tool".binaries]
tool-cli = "latest"
"#,
        );

        assert!(m.get("acme/tool", Some("tool-cli")).is_some());
        assert!(m.get("acme/tool", Some("tool-server")).is_none());
        // Asking for "the repo's one binary" is meaningless once it lists several.
        assert!(m.get("acme/tool", None).is_none());
    }

    #[test]
    fn covers_reports_whether_an_installed_binary_is_still_wanted() {
        let m = parse(
            r#"
[tools]
"sharkdp/bat" = "latest"

[tools."acme/tool".binaries]
tool-cli = "latest"
"#,
        );

        assert!(m.covers("sharkdp/bat", "bat"));
        // A repo naming no binaries wants whichever one is installed.
        assert!(m.covers("sharkdp/bat", ""));
        assert!(m.covers("acme/tool", "tool-cli"));
        assert!(!m.covers("acme/tool", "tool-server"));
        assert!(!m.covers("nobody/else", "else"));
    }

    #[test]
    fn writing_a_bare_repo_keeps_the_file_to_one_line() {
        let mut d = DocumentMut::new();
        set_spec(&mut d, "sharkdp/bat", None, spec_value(None, None)).unwrap();

        assert_eq!(d.to_string(), "[tools]\n\"sharkdp/bat\" = \"latest\"\n");
    }

    #[test]
    fn an_alias_promotes_the_value_to_an_inline_table() {
        let mut d = DocumentMut::new();
        set_spec(
            &mut d,
            "BurntSushi/ripgrep",
            None,
            spec_value(Some("14.1.0"), Some("rg")),
        )
        .unwrap();

        let out = d.to_string();
        assert!(
            out.contains(r#""BurntSushi/ripgrep" = { tag = "14.1.0", alias = "rg" }"#),
            "{out}"
        );
        assert_eq!(
            parse(&out)
                .get("BurntSushi/ripgrep", None)
                .unwrap()
                .alias
                .as_deref(),
            Some("rg")
        );
    }

    #[test]
    fn naming_a_binary_nests_it_under_the_repo() {
        let mut d = DocumentMut::new();
        set_spec(
            &mut d,
            "acme/tool",
            Some("tool-cli"),
            spec_value(None, None),
        )
        .unwrap();
        set_spec(
            &mut d,
            "acme/tool",
            Some("tool-server"),
            spec_value(Some("v1.1.0"), None),
        )
        .unwrap();

        let m = parse(&d.to_string());
        assert_eq!(m.iter().count(), 2);
        assert_eq!(
            m.is_pinned("acme/tool", Some("tool-server")).as_deref(),
            Some("v1.1.0")
        );
    }

    #[test]
    fn re_pinning_preserves_comments_and_other_entries() {
        let mut d = doc(r#"# my tools
[tools]
"BurntSushi/ripgrep" = "latest"
"sharkdp/bat" = "v0.24.0"   # pinned: v0.25 broke theme
"#);

        set_spec(
            &mut d,
            "sharkdp/bat",
            None,
            spec_value(Some("v0.25.0"), None),
        )
        .unwrap();

        let out = d.to_string();
        assert!(out.contains("# my tools"));
        assert!(out.contains("BurntSushi/ripgrep"));
        assert!(out.contains("# pinned: v0.25 broke theme"));
        assert!(out.contains("v0.25.0"));
        assert!(!out.contains("v0.24.0"));
    }

    #[test]
    fn clearing_a_pin_keeps_the_alias() {
        let mut d = doc(r#"[tools]
"BurntSushi/ripgrep" = { tag = "14.1.0", alias = "rg" }
"#);

        let alias = existing_alias(&d, "BurntSushi/ripgrep", None);
        set_spec(
            &mut d,
            "BurntSushi/ripgrep",
            None,
            spec_value(None, alias.as_deref()),
        )
        .unwrap();

        let m = parse(&d.to_string());
        let rg = m.get("BurntSushi/ripgrep", None).unwrap();
        assert_eq!(rg.tag, None);
        assert_eq!(rg.alias.as_deref(), Some("rg"));
    }

    #[test]
    fn removing_the_last_binary_removes_the_repo_with_it() {
        let mut d = doc(r#"[tools."acme/tool".binaries]
tool-cli = "latest"
tool-server = "latest"
"#);

        assert!(remove_spec(&mut d, "acme/tool", Some("tool-cli")));
        assert!(
            parse(&d.to_string())
                .get("acme/tool", Some("tool-server"))
                .is_some()
        );

        assert!(remove_spec(&mut d, "acme/tool", Some("tool-server")));
        assert!(parse(&d.to_string()).is_empty());
    }

    /// Removing one binary must not take its siblings, which is what dropping the whole
    /// repo row used to do.
    #[test]
    fn removing_one_binary_leaves_its_siblings_alone() {
        let mut d = doc(r#"[tools."acme/tool".binaries]
tool-cli = "latest"
tool-server = "v1.1.0"
"#);

        assert!(remove_spec(&mut d, "acme/tool", Some("tool-cli")));

        let m = parse(&d.to_string());
        assert!(m.get("acme/tool", Some("tool-cli")).is_none());
        assert_eq!(
            m.is_pinned("acme/tool", Some("tool-server")).as_deref(),
            Some("v1.1.0")
        );
    }

    #[test]
    fn removing_what_is_not_there_reports_false() {
        let mut d = doc(r#"[tools]
"a/b" = "latest"
"#);
        assert!(!remove_spec(&mut d, "missing/repo", None));
        assert!(!remove_spec(&mut d, "a/b", Some("nope")));
        assert!(remove_spec(&mut d, "a/b", None));
    }

    #[test]
    fn a_commented_out_entry_is_invisible_to_sync() {
        let m = parse(
            r#"[tools]
"BurntSushi/ripgrep" = "latest"
# kept for later
# "junegunn/fzf" = "latest"
"#,
        );
        assert_eq!(m.iter().count(), 1);
    }

    #[test]
    fn an_empty_manifest_round_trips() {
        let m = Manifest::default();
        let raw = toml::to_string_pretty(&m).unwrap();
        assert!(parse(&raw).is_empty());
    }
}
