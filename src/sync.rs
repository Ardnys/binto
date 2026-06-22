use std::path::PathBuf;

use anyhow::Result;
use tokio::task::JoinSet;

use crate::config::Config;
use crate::github::GithubClient;
use crate::install::{InstallRequest, ReleaseSelection};
use crate::manifest::{Manifest, ManifestEntry};
use crate::output::{print_info, print_success, print_warning};
use crate::state::State;

/// `ghr sync`: install every tool in the manifest that isn't already in local state.
/// Pinned entries install their exact tag; the rest install the latest release. Tools
/// already present are left untouched (re-versioning is `update`'s job, not `sync`'s).
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

    for entry in manifest.iter() {
        let repo = &entry.repo;

        if state.contains_repo(repo) {
            skipped += 1;
            continue;
        }

        let selection = match &entry.tag {
            Some(tag) => {
                print_info(&format!("Installing {repo} (pinned {tag})..."));
                ReleaseSelection::Tag(tag.clone())
            }
            None => {
                print_info(&format!("Installing {repo} (latest)..."));
                ReleaseSelection::Latest
            }
        };

        match (InstallRequest {
            repo,
            selection,
            install_dir: &config.install_dir,
            alias: entry.alias.as_deref(),
            include_prerelease: false,
        })
        .execute(&client, config, &mut state)
        .await
        {
            Ok(result) => {
                installed += 1;
                print_success(&format!(
                    "{} {} → {}",
                    result.tool_entry.binary_name,
                    result.tool_entry.installed_tag,
                    result.installed_path.display()
                ));
            }
            Err(e) => {
                failed += 1;
                print_warning(&format!("Failed to install {repo}: {e:#}"));
            }
        }
    }

    if installed > 0 {
        state.save()?;
    }

    println!();
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
/// confirms before deleting unless `yes` is set. Unlike `ghr remove`, this does NOT touch the
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

    println!();
    print_warning("These managed tools are not in the manifest and will be removed:");
    for (name, path) in &extras {
        println!("  {name} ({})", path.display());
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
