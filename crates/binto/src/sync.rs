use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use tokio::task::JoinSet;
use tracing::Instrument;

use crate::config::Config;
use crate::github::GithubClient;
use crate::github::types::{Asset, Release};
use crate::installer::download::download_span;
use crate::installer::{Downloaded, InstallSpec, default_binary_name};
use crate::manifest::{Manifest, ManifestEntry};
use crate::matcher::score::detect_arch;
use crate::output::{print_info, print_status, print_success, print_warning};
use crate::picker;
use crate::state::State;

/// Carries a manifest entry across the concurrent sync's phase boundaries: the download phase
/// (fanned out) produces a `Downloaded` keyed by `install_name`, which the sequential install
/// phase pairs back up here to build an `InstallSpec`.
struct PendingSync {
    /// Installed filename + state key: the `--alias` from the manifest, else the repo-derived
    /// default. Also the progress-bar label and the download/install map key.
    install_name: String,
    entry: ManifestEntry,
    release: Release,
    asset: Asset,
    install_dir: PathBuf,
}

/// `binto sync`: install every tool in the manifest that isn't already in local state.
/// Pinned entries install their exact tag; the rest install the latest release. Tools
/// already present are left untouched (re-versioning is `update`'s job, not `sync`'s).
///
/// Runs as a four-phase concurrent pipeline (mirroring `update --all`): fan-out release
/// resolution → sequential asset selection → concurrent downloads → sequential extract+install.
///
/// With `prune`, after installing, also remove managed tools whose repo is no longer in the
/// manifest (the manifest is the source of truth). `yes` skips the prune confirmation.
pub async fn cmd_sync(config: &Config, prune: bool, yes: bool) -> Result<()> {
    let manifest = Manifest::load()?;
    let mut state = State::load()?;

    // An empty manifest means "install nothing". Without --prune there's nothing to do; with
    // --prune it means "nothing should be managed", so fall through to the prune phase.
    if manifest.tools.is_empty() && !prune {
        print_info(&format!(
            "Manifest is empty ({}). Install a tool or add entries to it first.",
            Manifest::manifest_path().display()
        ));
        return Ok(());
    }

    let token = GithubClient::resolve_token(config.github_token.clone());
    let client = GithubClient::new(token)?;

    let mut installed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    // Phase A: concurrent release resolution. Pinned entries resolve their exact tag; the rest
    // resolve the newest non-draft release. Tools already in state are skipped up front so we
    // don't waste a request on them.
    let snapshot: Vec<ManifestEntry> = manifest.iter().cloned().collect();
    let mut api_set: JoinSet<(ManifestEntry, Result<Release>)> = JoinSet::new();

    for entry in snapshot {
        if state.contains_repo(&entry.repo) {
            skipped += 1;
            continue;
        }
        let client = client.clone();
        let repo = entry.repo.clone();
        let include_prerelease = config.include_prereleases;
        api_set.spawn(async move {
            let release: Result<Release> = match &entry.tag {
                Some(tag) => client.get_release_by_tag(&repo, tag).await,
                None => client.list_releases(&repo).await.and_then(|mut releases| {
                    if include_prerelease {
                        releases.retain(|r| !r.draft);
                    } else {
                        releases.retain(|r| !r.prerelease && !r.draft);
                    }
                    releases
                        .into_iter()
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("no releases found for {repo}"))
                }),
            };
            (entry, release)
        });
    }

    let mut api_results: Vec<(ManifestEntry, Result<Release>)> = Vec::new();
    while let Some(res) = api_set.join_next().await {
        api_results.push(res?);
    }
    api_results.sort_by(|a, b| a.0.repo.cmp(&b.0.repo));

    // Phase B: sequential asset selection (the interactive picker must stay on the main task).
    let user_arch = detect_arch();
    let mut pending: Vec<PendingSync> = Vec::new();

    for (entry, result) in api_results {
        let release = match result {
            Ok(release) => release,
            Err(e) => {
                failed += 1;
                print_warning(&format!("Failed to resolve {}: {e:#}", entry.repo));
                continue;
            }
        };

        let install_name = entry
            .alias
            .clone()
            .unwrap_or_else(|| default_binary_name(&entry.repo).to_string());

        match picker::select_asset(
            &release,
            &user_arch,
            None,
            &entry.repo,
            &format!("Pick an asset for {}", entry.repo),
            config.prefer_libc,
            false,
        ) {
            Ok(asset) => pending.push(PendingSync {
                install_name,
                entry,
                release,
                asset,
                install_dir: config.install_dir.clone(),
            }),
            Err(e) => {
                failed += 1;
                print_warning(&format!(
                    "Failed to pick an asset for {}: {e:#}",
                    entry.repo
                ));
            }
        }
    }

    if !pending.is_empty() {
        // Phase C: concurrent downloads. Each task runs inside its own `download` span, which
        // renders the byte-progress bar (via tracing-indicatif) and tags the task's log events.
        let http = client.http_client().clone();
        let mut pending_map: HashMap<String, PendingSync> = HashMap::new();
        let mut dl_set: JoinSet<(String, Result<Downloaded>)> = JoinSet::new();

        for p in pending {
            let task_name = p.install_name.clone();
            let http = http.clone();
            let asset = p.asset.clone();
            // Each task verifies against its own release's assets (the checksum lives there),
            // not the union of every tool's picked asset.
            let all_assets = p.release.assets.clone();
            let span = download_span(&task_name, asset.size);
            dl_set.spawn(
                async move {
                    (
                        task_name,
                        Downloaded::fetch(&http, &asset, &all_assets).await,
                    )
                }
                .instrument(span),
            );
            pending_map.insert(p.install_name.clone(), p);
        }

        let mut downloads: Vec<(String, Downloaded)> = Vec::new();
        while let Some(res) = dl_set.join_next().await {
            let (name, dl_result) = res?;
            match dl_result {
                Ok(dl) => downloads.push((name, dl)),
                Err(e) => {
                    pending_map.remove(&name);
                    failed += 1;
                    print_warning(&format!("Failed to download {name}: {e:#}"));
                }
            }
        }

        // Phase D: sequential extract + install (handles the interactive binary picker safely).
        for (name, dl) in downloads {
            let Some(p) = pending_map.remove(&name) else {
                continue;
            };
            // The archive still ships the upstream-named binary; the builder locates it by the
            // repo-derived name, but installs under the tracked name — which may be an `--alias`.
            let spec = InstallSpec::builder(&p.entry.repo, &p.release, &p.asset)
                .install_dir(&p.install_dir)
                .install_name(&p.install_name)
                .build();
            match spec.install(dl) {
                Ok(ir) => {
                    installed += 1;
                    print_success(&format!(
                        "{} {} → {}",
                        ir.tool_entry.binary_name,
                        ir.tool_entry.installed_tag,
                        ir.installed_path.display()
                    ));
                    state.upsert(ir.tool_entry);
                }
                Err(e) => {
                    failed += 1;
                    print_warning(&format!("Failed to install {name}: {e:#}"));
                }
            }
        }
    }

    if installed > 0 {
        state.save()?;
    }

    print_status("");
    print_info(&format!(
        "Sync complete: {installed} installed, {skipped} already present, {failed} failed."
    ));

    if prune {
        prune_extras(&mut state, &manifest, yes)?;
    }

    Ok(())
}

/// Remove managed tools whose repo is no longer in the manifest (the manifest is the source
/// of truth). State is keyed by binary name, the manifest by repo. Lists the candidates and
/// confirms before deleting unless `yes` is set. Unlike `binto remove`, this does NOT touch the
/// manifest — the tools are already absent from it, which is exactly why they're pruned.
fn prune_extras(state: &mut State, manifest: &Manifest, yes: bool) -> Result<()> {
    let extras: Vec<(String, PathBuf)> = state
        .iter()
        .filter(|(_, e)| manifest.get(&e.repo).is_none())
        .map(|(name, e)| (name.clone(), e.install_path.clone()))
        .collect();

    if extras.is_empty() {
        print_info("Nothing to prune — every managed tool is in the manifest.");
        return Ok(());
    }

    print_status("");
    print_warning("These managed tools are not in the manifest and will be removed:");
    for (name, path) in &extras {
        print_status(&format!("  {name} ({})", path.display()));
    }

    let confirmed = yes
        || dialoguer::Confirm::new()
            .with_prompt(format!("Remove {} tool(s)?", extras.len()))
            .default(false)
            .interact()?;
    if !confirmed {
        print_info("Prune cancelled.");
        return Ok(());
    }

    let mut pruned = 0usize;
    for (name, _) in &extras {
        match crate::remove_tool(state, name) {
            Ok(_) => {
                pruned += 1;
                print_success(&format!("Removed {name}."));
            }
            Err(e) => print_warning(&format!("Failed to remove {name}: {e:#}")),
        }
    }

    if pruned > 0 {
        state.save()?;
    }
    print_info(&format!("Pruned {pruned} tool(s)."));

    Ok(())
}
