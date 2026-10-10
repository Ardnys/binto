use std::path::PathBuf;

use anyhow::Result;

use crate::config::Config;
use crate::installer::plan::{ReleaseSource, Target};
use crate::installer::run::{Ctx, Outcome, run};
use crate::manifest::{Manifest, ManifestEntry};
use crate::output::{print_info, print_status, print_success, print_warning};
use crate::reporter::{DownloadReporter, TermPrompter, report};
use crate::state::State;

/// `binto sync`: install every tool in the manifest that isn't already in local state.
/// Pinned entries install their exact tag; the rest install the latest release. Tools
/// already present are left untouched (re-versioning is `update`'s job, not `sync`'s).
///
/// With `prune`, after installing, also remove managed tools whose repo is no longer in the
/// manifest (the manifest is the source of truth). `yes` skips the prune confirmation.
pub async fn cmd_sync(config: &Config, prune: bool, yes: bool) -> Result<()> {
    let manifest = Manifest::load()?;

    // An empty manifest means "install nothing". Without --prune there's nothing to do; with
    // --prune it means "nothing should be managed", so fall through to the prune phase.
    if manifest.is_empty() && !prune {
        print_info(&format!(
            "Manifest is empty ({}). Install a tool or add entries to it first.",
            Manifest::manifest_path().display()
        ));
        return Ok(());
    }

    let state = State::load()?;
    let (present, missing): (Vec<_>, Vec<_>) = manifest
        .iter()
        .partition(|row| state.has_binary(&row.repo, row.binary.as_deref()));
    let targets = missing.into_iter().map(|row| target(row, config)).collect();

    let reporter = DownloadReporter::default();
    let ctx = Ctx::new(config, &reporter, &TermPrompter)?;
    let outcomes = run(targets, &ctx).await?;
    let count = |f: fn(&Outcome) -> bool| outcomes.iter().filter(|o| f(o)).count();
    let installed = count(|o| matches!(o, Outcome::Installed(_)));
    let failed = count(|o| matches!(o, Outcome::Failed { .. }));
    let reported = report(outcomes);

    print_status("");
    print_info(&format!(
        "Sync complete: {installed} installed, {} already present, {failed} failed.",
        present.len()
    ));

    if prune {
        prune_extras(&manifest, yes)?;
    }
    reported.map(drop)
}

/// A manifest row binto hasn't installed: its pin if it has one, else the latest release,
/// under its alias.
fn target(row: ManifestEntry, config: &Config) -> Target {
    let source = match row.tag {
        Some(tag) => ReleaseSource::Tag(tag),
        None => ReleaseSource::Latest {
            include_prerelease: config.include_prereleases,
        },
    };
    Target::fresh(row.repo, source, row.alias, config.install_dir.clone())
}

/// Remove managed tools whose repo is no longer in the manifest (the manifest is the source
/// of truth). State is keyed by binary name, the manifest by repo. Lists the candidates and
/// confirms before deleting unless `yes` is set. Unlike `binto remove`, this does NOT touch the
/// manifest — the tools are already absent from it, which is exactly why they're pruned.
fn prune_extras(manifest: &Manifest, yes: bool) -> Result<()> {
    let state = State::load()?;
    let extras: Vec<(String, PathBuf)> = state
        .iter()
        .filter(|(_, e)| !manifest.covers(&e.repo, &e.stem))
        .map(|(name, e)| (name.to_owned(), e.install_path.clone()))
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

    let confirmed = crate::reporter::prompter(yes)
        .confirm(&format!("Remove {} tool(s)?", extras.len()), false)?;
    if !confirmed {
        print_info("Prune cancelled.");
        return Ok(());
    }

    let mut pruned = 0usize;
    for (name, _) in &extras {
        match State::try_mutate(|s| crate::remove_tool(s, name)) {
            Ok(_) => {
                pruned += 1;
                print_success(&format!("Removed {name}."));
            }
            Err(e) => print_warning(&format!("Failed to remove {name}: {e:#}")),
        }
    }

    print_info(&format!("Pruned {pruned} tool(s)."));

    Ok(())
}
