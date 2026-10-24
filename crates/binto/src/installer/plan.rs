//! What to install, decided before anything is downloaded. Every command describes its work
//! as [`Target`]s; [`resolve`] turns one into a release and [`spec_for`] into the
//! [`InstallSpec`] the installer runs. An update is a target that carries the
//! [`ToolEntry`] it replaces — which is the only thing that sets it apart from an install.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::Libc;
use crate::error::BintoError;
use crate::github::GithubClient;
use crate::github::api::{ApiResponse, ConditionalResult};
use crate::github::types::Release;
use crate::installer::reporter::Prompter;
use crate::installer::{InstallSpec, default_binary_name};
use crate::matcher::facts::{AssetName, detect_arch};
use crate::output::{print_info, print_warning};
use crate::picker;
use crate::state::{State, ToolEntry};

/// Which release a [`Target`] asks for.
pub enum ReleaseSource {
    /// This exact tag: `-t`, or a pinned manifest row.
    Tag(String),
    /// The newest release.
    Latest { include_prerelease: bool },
    /// Whichever release the user picks from the list.
    Pick { include_prerelease: bool },
    /// GitHub's latest release, but only when it is newer than the installed one. Asked
    /// conditionally on the installed entry's ETag, so an unchanged repo costs a `304`.
    Newer,
}

/// One tool to install or update, before any network call.
pub struct Target {
    repo: String,
    source: ReleaseSource,
    /// The tracked tool this replaces. `Some` makes this an update: the asset is picked like
    /// the last one, and the name and directory are kept.
    existing: Option<ToolEntry>,
    /// The `--alias` of a fresh install.
    alias: Option<String>,
    install_dir: PathBuf,
    /// Stems of the repo's binaries already tracked, which a fresh install must not pick.
    skip_stems: Vec<String>,
    /// Ask before taking over a binary of the same name found on `$PATH`.
    confirm_unmanaged: bool,
}

/// What [`resolve`] found.
pub enum Resolution {
    /// Nothing newer than what is installed. `latest` is the tag GitHub reported, when it
    /// sent one (a `304` sends nothing).
    UpToDate { latest: Option<String> },
    /// Install this release.
    Release {
        release: Release,
        etag: Option<String>,
    },
}

impl Target {
    /// A tool binto doesn't track yet.
    pub fn fresh(
        repo: impl Into<String>,
        source: ReleaseSource,
        alias: Option<String>,
        install_dir: PathBuf,
    ) -> Self {
        Self {
            repo: repo.into(),
            source,
            existing: None,
            alias,
            install_dir,
            skip_stems: Vec::new(),
            confirm_unmanaged: false,
        }
    }

    /// Replace a tracked tool, in its own directory and under its own name.
    pub fn update(entry: ToolEntry, source: ReleaseSource, default_dir: &Path) -> Self {
        Self {
            repo: entry.repo.clone(),
            install_dir: entry.install_dir(default_dir).to_path_buf(),
            source,
            existing: Some(entry),
            alias: None,
            skip_stems: Vec::new(),
            confirm_unmanaged: false,
        }
    }

    /// The target for `binto install <repo>`, checked against what's already tracked before
    /// any request is made. A tracked tool given `-t` becomes an update to that tag (a
    /// re-pin); without one, installing it again is an error — that is `binto update`'s job.
    pub fn for_install(
        state: &State,
        repo: &str,
        source: ReleaseSource,
        alias: Option<String>,
        install_dir: PathBuf,
        default_dir: &Path,
    ) -> Result<Self> {
        let repin = matches!(source, ReleaseSource::Tag(_));

        if let Some(name) = alias.as_deref() {
            return match state.get(name) {
                None => Ok(Self::fresh(repo, source, alias, install_dir).confirming_unmanaged()),
                Some(e) if e.repo != repo => {
                    bail!("'{name}' is already taken by {} in binto", e.repo)
                }
                Some(e) if repin => Ok(Self::repin(e, source, &install_dir, default_dir)),
                Some(e) => Err(already_managed(e)),
            };
        }

        let tracked: Vec<&ToolEntry> = state
            .iter()
            .map(|(_, e)| e)
            .filter(|e| e.repo == repo)
            .collect();
        // A repo shipping one binary can only mean that one again. One shipping several may
        // mean another of its binaries, so it goes on to the picker without the tracked ones.
        let only_binaries = tracked.iter().all(|e| is_only_binary(e));
        match tracked.as_slice() {
            [] => Ok(Self::fresh(repo, source, None, install_dir).confirming_unmanaged()),
            [one] if repin && only_binaries => {
                Ok(Self::repin(one, source, &install_dir, default_dir))
            }
            [..] if only_binaries && repin => bail!(
                "{repo} is tracked several times ({}); pass `--alias <name>` to say which to re-pin",
                names(&tracked)
            ),
            [..] if only_binaries => Err(already_managed(tracked[0])),
            [..] => Ok(Self {
                skip_stems: tracked.iter().map(|e| e.stem.clone()).collect(),
                ..Self::fresh(repo, source, None, install_dir).confirming_unmanaged()
            }),
        }
    }

    /// Move a tracked tool to another tag, in place. Its directory is kept, so a `--to`
    /// pointing elsewhere is ignored — say so rather than leave the user guessing.
    fn repin(
        entry: &ToolEntry,
        source: ReleaseSource,
        install_dir: &Path,
        default_dir: &Path,
    ) -> Self {
        let target = Self::update(entry.clone(), source, default_dir);
        if install_dir != default_dir && install_dir != target.install_dir {
            print_warning(&format!(
                "{} stays in {} when re-pinned; --to is ignored.",
                entry.binary_name,
                target.install_dir.display()
            ));
        }
        target
    }

    fn confirming_unmanaged(mut self) -> Self {
        self.confirm_unmanaged = true;
        self
    }

    /// The name to show for this tool before its asset (and so maybe its name) is known.
    pub fn label(&self) -> &str {
        match (&self.existing, &self.alias) {
            (Some(e), _) => &e.binary_name,
            (None, Some(alias)) => alias,
            (None, None) => &self.repo,
        }
    }

    /// Whether resolving this target asks the user something (the release picker), so it
    /// must not run alongside others.
    pub fn prompts(&self) -> bool {
        matches!(self.source, ReleaseSource::Pick { .. })
    }

    pub fn existing(&self) -> Option<&ToolEntry> {
        self.existing.as_ref()
    }

    pub fn confirm_unmanaged(&self) -> bool {
        self.confirm_unmanaged
    }

    /// The release's assets this target may pick from: all of them, less the binaries the
    /// repo already has tracked.
    fn candidates<'r>(&self, release: &'r Release) -> Cow<'r, Release> {
        if self.skip_stems.is_empty() {
            return Cow::Borrowed(release);
        }
        let mut release = release.clone();
        release.assets.retain(|a| {
            let stem = AssetName::new(&a.name).stem(Some(&release.tag_name));
            !self.skip_stems.contains(&stem)
        });
        Cow::Owned(release)
    }
}

/// The error for installing a tool that is already tracked.
fn already_managed(entry: &ToolEntry) -> anyhow::Error {
    let name = &entry.binary_name;
    anyhow::anyhow!(
        "'{name}' is already managed by binto ({}). Run `binto update {name}` to upgrade it, \
         or pass `-t <tag>` to re-pin it to a specific release.",
        entry.installed_tag
    )
}

/// Whether `entry` is its repo's only binary, so a second install of the repo can only mean
/// the same one again. A tool named by its stem came from a release shipping several.
fn is_only_binary(entry: &ToolEntry) -> bool {
    let default = default_binary_name(&entry.repo);
    entry.stem.is_empty() || entry.stem == default || entry.binary_name == default
}

fn names(entries: &[&ToolEntry]) -> String {
    entries
        .iter()
        .map(|e| e.binary_name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Drop drafts, and prereleases unless asked for.
pub fn filter_releases(releases: &mut Vec<Release>, include_prerelease: bool) {
    releases.retain(|r| !r.draft && (include_prerelease || !r.prerelease));
}

/// Find the release `target` asks for. May prompt, for [`ReleaseSource::Pick`].
pub async fn resolve(
    client: &GithubClient,
    target: &Target,
    prompter: &dyn Prompter,
) -> Result<Resolution> {
    let repo = &target.repo;
    let release = match &target.source {
        ReleaseSource::Tag(tag) => client.get_release_by_tag(repo, tag).await?,
        ReleaseSource::Latest { include_prerelease } => releases(client, repo, *include_prerelease)
            .await?
            .swap_remove(0),
        ReleaseSource::Pick { include_prerelease } => {
            pick_release(releases(client, repo, *include_prerelease).await?, prompter)?
        }
        ReleaseSource::Newer => return newer(client, target).await,
    };
    Ok(Resolution::Release {
        release,
        etag: None,
    })
}

/// The repo's installable releases, newest first. Never empty.
async fn releases(
    client: &GithubClient,
    repo: &str,
    include_prerelease: bool,
) -> Result<Vec<Release>> {
    let mut releases = client.list_releases(repo).await?;
    filter_releases(&mut releases, include_prerelease);
    if releases.is_empty() {
        bail!("no releases found for {repo}");
    }
    Ok(releases)
}

fn pick_release(mut releases: Vec<Release>, prompter: &dyn Prompter) -> Result<Release> {
    let labels: Vec<String> = releases
        .iter()
        .map(|r| format!("{} ({})", r.tag_name, r.published_at.format("%Y-%m-%d")))
        .collect();
    let idx = prompter.fuzzy_select("Pick a release", &labels)?;
    anyhow::ensure!(idx < releases.len(), "release pick {idx} is out of range");
    let release = releases.swap_remove(idx);
    print_info(&format!(
        "Selected: {} — {}",
        release.tag_name, release.html_url
    ));
    Ok(release)
}

/// GitHub's latest release, when it is newer than the installed one.
async fn newer(client: &GithubClient, target: &Target) -> Result<Resolution> {
    let existing = target
        .existing
        .as_ref()
        .context("only a tracked tool can look for a newer release")?;
    let result = client
        .get_latest_release(&target.repo, existing.etag.as_deref())
        .await?;
    Ok(match result {
        ConditionalResult::NotModified => Resolution::UpToDate { latest: None },
        ConditionalResult::Changed(ApiResponse { data, etag }) => {
            if existing.is_behind(data.published_at) {
                Resolution::Release {
                    release: data,
                    etag,
                }
            } else {
                Resolution::UpToDate {
                    latest: Some(data.tag_name),
                }
            }
        }
    })
}

/// Pick the asset to install from `release` and settle what it is called and where it goes.
/// May prompt for the asset.
///
/// An update picks like its last install (`previous`) and keeps the tracked name and
/// directory; a fresh install is named by its `--alias`, else by the binary the asset turns
/// out to be.
pub fn spec_for(
    target: &Target,
    release: Release,
    etag: Option<String>,
    prefer_libc: Libc,
    prompter: &dyn Prompter,
) -> Result<InstallSpec> {
    let existing = target.existing.as_ref();
    let selected = picker::select_asset(
        &target.candidates(&release),
        &detect_arch(),
        existing.map(ToolEntry::previous_choice),
        &target.repo,
        &format!("Pick an asset for {}", target.label()),
        prefer_libc,
        prompter,
    )
    .map_err(|e| explain_no_asset(target, e))?;

    let default = default_binary_name(&target.repo);
    let recorded_stem = existing
        .map(|e| e.stem.as_str())
        .filter(|stem| !stem.is_empty());
    let find_name = recorded_stem
        .or(selected.variant.as_deref())
        .unwrap_or(default)
        .to_string();
    let install_name = existing
        .map(|e| e.binary_name.as_str())
        .or(target.alias.as_deref())
        .or(selected.variant.as_deref())
        .unwrap_or(default)
        .to_string();

    Ok(InstallSpec {
        repo: target.repo.clone(),
        asset: selected.asset,
        release,
        install_dir: target.install_dir.clone(),
        find_name,
        install_name,
        variant: selected.variant,
        etag,
        replaces: existing.cloned(),
    })
}

/// With the tracked binaries filtered out, finding no asset means every one the release
/// ships is installed already.
fn explain_no_asset(target: &Target, err: anyhow::Error) -> anyhow::Error {
    let nothing_left = matches!(
        err.downcast_ref::<BintoError>(),
        Some(BintoError::NoCompatibleAssets { .. })
    );
    if target.skip_stems.is_empty() || !nothing_left {
        return err;
    }
    let hint = match target.source {
        ReleaseSource::Tag(_) => "Pass `--alias <name>` to re-pin one of them",
        _ => "Run `binto update` to upgrade them",
    };
    anyhow::anyhow!(
        "every binary {} ships is already managed by binto ({}). {hint}.",
        target.repo,
        target.skip_stems.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::github::types::Asset;
    use crate::reporter::AssumeYes;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
            size: 1024,
            content_type: "application/gzip".to_string(),
        }
    }

    fn release(tag: &str, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.to_string(),
            published_at: DateTime::<Utc>::UNIX_EPOCH,
            prerelease: false,
            draft: false,
            assets: assets.iter().map(|n| asset(n)).collect(),
            html_url: String::new(),
        }
    }

    fn entry(repo: &str, name: &str, stem: &str, path: &str) -> ToolEntry {
        ToolEntry {
            repo: repo.to_string(),
            installed_tag: "v1.0.0".to_string(),
            install_path: PathBuf::from(path),
            binary_name: name.to_string(),
            stem: stem.to_string(),
            asset_pattern: String::new(),
            installed_sha256: None,
            etag: None,
            last_checked: None,
            published_at: None,
        }
    }

    fn state(entries: &[ToolEntry]) -> State {
        let mut state = State::default();
        for e in entries {
            state.upsert(e.clone());
        }
        state
    }

    fn latest() -> ReleaseSource {
        ReleaseSource::Latest {
            include_prerelease: false,
        }
    }

    fn bin() -> PathBuf {
        PathBuf::from("/home/u/.local/bin")
    }

    const FD: &[&str] = &[
        "fd-v10.2.0-x86_64-unknown-linux-gnu.tar.gz",
        "fd-v10.2.0-x86_64-unknown-linux-musl.tar.gz",
    ];

    const LUTGEN: &[&str] = &[
        "lutgen-cli-v1.2.0-x86_64-unknown-linux-gnu.tar.gz",
        "lutgen-studio-v1.2.0-x86_64-unknown-linux-gnu.tar.gz",
    ];

    fn spec(target: &Target, rel: Release) -> InstallSpec {
        spec_for(target, rel, None, Libc::Gnu, &AssumeYes).unwrap()
    }

    #[test]
    fn a_fresh_install_is_named_after_the_repo() {
        let target = Target::fresh("sharkdp/fd", latest(), None, bin());
        let s = spec(&target, release("v10.2.0", FD));
        assert_eq!(s.install_name, "fd");
        assert_eq!(s.find_name, "fd");
        assert_eq!(s.install_dir, bin());
        assert!(s.replaces.is_none());
    }

    #[test]
    fn an_alias_names_the_install_but_not_the_archive_lookup() {
        let target = Target::fresh("sharkdp/fd", latest(), Some("fdfind".into()), bin());
        let s = spec(&target, release("v10.2.0", FD));
        assert_eq!(s.install_name, "fdfind");
        assert_eq!(s.find_name, "fd");
    }

    #[test]
    fn an_update_keeps_the_tracked_name_and_directory() {
        let e = entry("sharkdp/fd", "fdfind", "fd", "/usr/local/bin/fdfind");
        let target = Target::update(e, ReleaseSource::Newer, &bin());
        let s = spec(&target, release("v10.2.0", FD));
        assert_eq!(s.install_name, "fdfind");
        assert_eq!(s.find_name, "fd");
        assert_eq!(s.install_dir, PathBuf::from("/usr/local/bin"));
        assert_eq!(s.replaces.unwrap().installed_tag, "v1.0.0");
    }

    #[test]
    fn a_second_install_of_a_one_binary_repo_fails_before_any_request() {
        let st = state(&[entry("sharkdp/fd", "fd", "fd", "/b/fd")]);
        let err = Target::for_install(&st, "sharkdp/fd", latest(), None, bin(), &bin())
            .err()
            .unwrap();
        assert!(err.to_string().contains("already managed"), "{err}");
    }

    #[test]
    fn a_tag_on_a_tracked_tool_re_pins_it_in_place() {
        let st = state(&[entry("sharkdp/fd", "fd", "fd", "/opt/bin/fd")]);
        let tag = ReleaseSource::Tag("v9.0.0".into());
        let target = Target::for_install(&st, "sharkdp/fd", tag, None, bin(), &bin()).unwrap();
        assert_eq!(target.existing().unwrap().binary_name, "fd");
        assert_eq!(target.install_dir, PathBuf::from("/opt/bin"));
        assert!(!target.confirm_unmanaged());
    }

    #[test]
    fn an_aliased_one_binary_repo_still_fails_before_any_request() {
        let st = state(&[entry("sharkdp/fd", "fdfind", "fd", "/b/fdfind")]);
        let err = Target::for_install(&st, "sharkdp/fd", latest(), None, bin(), &bin())
            .err()
            .unwrap();
        assert!(err.to_string().contains("already managed"), "{err}");
    }

    /// `-t` on a repo shipping several binaries may name the one not installed yet, so it
    /// must not quietly re-pin the tracked one.
    #[test]
    fn a_tag_on_a_multi_binary_repo_installs_an_untracked_binary() {
        let st = state(&[entry(
            "ozwaldorf/lutgen",
            "lutgen-cli",
            "lutgen-cli",
            "/b/x",
        )]);
        let tag = ReleaseSource::Tag("v1.2.0".into());
        let target =
            Target::for_install(&st, "ozwaldorf/lutgen", tag, None, bin(), &bin()).unwrap();
        assert!(target.existing().is_none());
        let s = spec(&target, release("v1.2.0", LUTGEN));
        assert_eq!(s.asset.name, LUTGEN[1]);
    }

    #[test]
    fn an_alias_taken_by_another_repo_is_refused() {
        let st = state(&[entry("sharkdp/fd", "fd", "fd", "/b/fd")]);
        let err = Target::for_install(&st, "other/fd", latest(), Some("fd".into()), bin(), &bin())
            .err()
            .unwrap();
        assert!(err.to_string().contains("already taken"), "{err}");
    }

    #[test]
    fn a_multi_binary_repo_offers_only_untracked_binaries() {
        let st = state(&[entry(
            "ozwaldorf/lutgen",
            "lutgen-cli",
            "lutgen-cli",
            "/b/x",
        )]);
        let target =
            Target::for_install(&st, "ozwaldorf/lutgen", latest(), None, bin(), &bin()).unwrap();
        let s = spec(&target, release("v1.2.0", LUTGEN));
        assert_eq!(s.asset.name, LUTGEN[1]);
    }

    #[test]
    fn a_multi_binary_repo_with_everything_tracked_says_so() {
        let st = state(&[
            entry("ozwaldorf/lutgen", "lutgen-cli", "lutgen-cli", "/b/x"),
            entry("ozwaldorf/lutgen", "lutgen-studio", "lutgen-studio", "/b/y"),
        ]);
        let target =
            Target::for_install(&st, "ozwaldorf/lutgen", latest(), None, bin(), &bin()).unwrap();
        let err = spec_for(
            &target,
            release("v1.2.0", LUTGEN),
            None,
            Libc::Gnu,
            &AssumeYes,
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("already managed"), "{err}");
    }

    #[test]
    fn prereleases_and_drafts_are_dropped_unless_asked_for() {
        let mut rels = vec![release("v2", &[]), release("v1", &[]), release("v0", &[])];
        rels[0].prerelease = true;
        rels[1].draft = true;
        let mut all = rels.clone();
        filter_releases(&mut rels, false);
        assert_eq!(rels.len(), 1);
        filter_releases(&mut all, true);
        assert_eq!(all.len(), 2);
    }
}
