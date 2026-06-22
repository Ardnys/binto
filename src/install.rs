use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::{self, Config};
use crate::github::GithubClient;
use crate::github::types::Release;
use crate::installer::{InstallResult, InstallSpec, default_binary_name};
use crate::manifest::Manifest;
use crate::matcher::score::detect_arch;
use crate::output::{print_info, print_success, print_warning};
use crate::picker;
use crate::state::State;

/// How an [`InstallRequest`] should choose which release to install.
pub enum ReleaseSelection {
    /// Install this exact tag (version pinning). Resolved via `get_release_by_tag`.
    Tag(String),
    /// Show the interactive release picker (the default `ghr install` flow).
    InteractivePick,
}

fn filter_releases(releases: &mut Vec<Release>, include_prerelease: bool, config: &Config) {
    if !include_prerelease && !config.include_prereleases {
        releases.retain(|r| !r.prerelease && !r.draft);
    } else {
        releases.retain(|r| !r.draft);
    }
}

/// A high-level "install this repo" request: everything needed to go from a repo identity to
/// an installed, state-tracked tool. The unifying entry point for both `install` and `sync`.
pub struct InstallRequest<'a> {
    pub repo: &'a str,
    pub selection: ReleaseSelection,
    pub install_dir: &'a Path,
    pub alias: Option<&'a str>,
    pub include_prerelease: bool,
}

impl InstallRequest<'_> {
    /// Resolve the release per `selection`, pick the matching asset, download + install it,
    /// and upsert the result into `state`. Does NOT save state or touch the manifest — callers
    /// own that so they can batch writes.
    pub async fn execute(
        self,
        client: &GithubClient,
        config: &Config,
        state: &mut State,
    ) -> Result<InstallResult> {
        let release = self.resolve_release(client, config).await?;

        let user_arch = detect_arch();
        let asset = picker::select_asset(&release, &user_arch, None, self.repo, "Pick an asset")?;

        let mut builder =
            InstallSpec::builder(self.repo, &release, &asset).install_dir(self.install_dir);
        if let Some(alias) = self.alias {
            builder = builder.install_name(alias);
        }
        let result = builder.build().run(client.http_client()).await?;

        state.upsert(result.tool_entry.clone());
        Ok(result)
    }

    async fn resolve_release(&self, client: &GithubClient, config: &Config) -> Result<Release> {
        let release = match &self.selection {
            ReleaseSelection::Tag(tag) => client.get_release_by_tag(self.repo, tag).await?,
            ReleaseSelection::InteractivePick => {
                let mut releases = client.list_releases(self.repo).await?;
                filter_releases(&mut releases, self.include_prerelease, config);
                if releases.is_empty() {
                    anyhow::bail!("no releases found for {}", self.repo);
                }

                let labels: Vec<String> = releases
                    .iter()
                    .map(|r| format!("{} ({})", r.tag_name, r.published_at.format("%Y-%m-%d")))
                    .collect();
                let idx = dialoguer::FuzzySelect::new()
                    .with_prompt("Pick a release")
                    .items(&labels)
                    .default(0)
                    .interact()?;

                let release = releases.swap_remove(idx);
                println!("Selected: {} — {}", release.tag_name, release.html_url);
                release
            }
        };
        Ok(release)
    }
}

/// `ghr install <owner/repo> [-t <tag>]`. Pins to `tag` when given (and records the pin in
/// the manifest), otherwise shows the interactive release picker.
pub async fn cmd_install(
    repo: &str,
    tag: Option<String>,
    alias: Option<String>,
    to: Option<PathBuf>,
    include_prerelease: bool,
    config: &Config,
) -> Result<()> {
    // Resolve the effective install directory: a `--to` override (with `~` expanded) wins,
    // otherwise the configured install_dir. The chosen dir is stored in the tool's
    // install_path, so future `ghr update`s reinstall here too — same as adopted tools.
    let install_dir = match &to {
        Some(p) => config::expand_tilde(p),
        None => config.install_dir.clone(),
    };

    // Look for an already-managed tool under its install name (the `--alias`, or the
    // repo-derived default) before doing any network I/O.
    let mut state = State::load()?;
    let install_name = alias
        .as_deref()
        .unwrap_or_else(|| default_binary_name(repo));
    let already_managed = state
        .get(install_name)
        .is_some_and(|existing| existing.repo == repo);

    if already_managed {
        // Re-installing a managed tool only makes sense to move its pin, so require an
        // explicit tag. A bare `ghr install <repo>` must not silently reinstall — that's
        // what `ghr update` is for.
        let Some(new_tag) = tag.as_deref() else {
            let existing = state.get(install_name).unwrap();
            anyhow::bail!(
                "'{install_name}' is already managed by ghr ({}). \
                     Run `ghr update {install_name}` to upgrade it, \
                     or pass `-t <tag>` to re-pin it to a specific release.",
                existing.installed_tag
            );
        };
        // Re-point in place: reinstall at the new tag and move the manifest pin below. Skip
        // the on-PATH adoption prompt — the binary it would find is ours.
        // WARN: For an adopted tool that lives in some other directory (e.g. /usr/local/bin),
        // re-pointing with -t would place the new binary in install_dir and leave the original behind.
        print_info(&format!("Re-pointing {install_name} to {new_tag}."));
    } else if let Some(existing_path) = crate::find_on_path(install_name) {
        // Not managed by ghr, but a binary with this name already exists on $PATH.
        print_warning(&format!(
            "'{install_name}' is already installed at {}",
            existing_path.display()
        ));
        let proceed = dialoguer::Confirm::new()
            .with_prompt("Install anyway and let ghr manage it going forward?")
            .default(false)
            .interact()?;
        if !proceed {
            print_info("Installation cancelled.");
            return Ok(());
        }
    }

    let token = GithubClient::resolve_token(config.github_token.clone());
    let client = GithubClient::new(token)?;

    let selection = match tag.clone() {
        Some(t) => ReleaseSelection::Tag(t),
        None => ReleaseSelection::InteractivePick,
    };

    let result = InstallRequest {
        repo,
        selection,
        install_dir: &install_dir,
        alias: alias.as_deref(),
        include_prerelease,
    }
    .execute(&client, config, &mut state)
    .await?;
    state.save()?;

    print_success(&format!(
        "Installed {} {} → {}",
        install_name,
        result.tool_entry.installed_tag,
        result.installed_path.display()
    ));

    // Keep the declarative manifest in sync. The tag (Some => pinned, None => clears any
    // existing pin) and the alias are recorded against the repo so `ghr sync` can replay
    // both elsewhere. Format-preserving write: comments/ordering elsewhere are kept.
    Manifest::record_and_save(repo, tag.as_deref(), alias.as_deref())?;

    // Warn if the install dir is not on PATH
    // BUG: it looks weird with "." as a path
    if let Ok(path_var) = std::env::var("PATH") {
        let on_path = path_var.split(':').any(|p| Path::new(p) == install_dir);
        if !on_path {
            print_warning(&format!(
                "{} is not on your PATH. Add it: export PATH=\"{}:$PATH\"",
                install_dir.display(),
                install_dir.display()
            ));
        }
    }

    Ok(())
}
