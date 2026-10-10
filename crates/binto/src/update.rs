use anyhow::Result;

use crate::config::Config;
use crate::installer::plan::{ReleaseSource, Resolution, Target};
use crate::installer::run::{Ctx, Outcome, Resolved, record_checked, resolve_all, run};
use crate::manifest::Manifest;
use crate::output::{print_info, print_status, print_warning};
use crate::reporter::{DownloadReporter, TermPrompter, print_up_to_date, report};
use crate::state::{State, ToolEntry};

const NOTHING_MANAGED: &str =
    "No tools managed by binto. Run `binto install <owner/repo>` to get started.";

/// `binto update <name>` or `binto update --all`.
pub async fn cmd_update(
    name: Option<String>,
    all: bool,
    force: bool,
    config: &Config,
) -> Result<()> {
    let state = State::load()?;
    if state.is_empty() {
        print_info(NOTHING_MANAGED);
        return Ok(());
    }
    if all {
        if force {
            print_info(
                "--force has no effect with --all; pinned tools stay locked. \
                 Name a tool to force-update it.",
            );
        }
        return update_all(state, config).await;
    }
    let name = name.ok_or_else(|| anyhow::anyhow!("specify a tool name or pass --all"))?;
    update_one(state.require(&name)?.clone(), force, config).await
}

/// Update every tool the manifest doesn't pin. A pin is a lock, so `--all` leaves pinned
/// tools alone without spending a request on them.
async fn update_all(state: State, config: &Config) -> Result<()> {
    let manifest = Manifest::load()?;
    let mut pinned = Vec::new();
    let mut targets = Vec::new();
    for (_, entry) in state.iter() {
        match manifest.is_pinned(&entry.repo, entry.binary()) {
            Some(pin) => pinned.push(Outcome::Pinned {
                name: entry.binary_name.clone(),
                installed: entry.installed_tag.clone(),
                pin,
            }),
            None => targets.push(newer(entry.clone(), config)),
        }
    }

    let reporter = DownloadReporter::default();
    let ctx = Ctx::new(config, &reporter, &TermPrompter)?;
    let mut outcomes = run(targets, &ctx).await?;
    outcomes.extend(pinned);
    Outcome::sort(&mut outcomes);
    report(outcomes)?;
    Ok(())
}

/// Update one tool. A pinned one needs `--force`, which takes it back to the latest release
/// and clears the pin once that succeeded — the common case being "I pinned an older
/// version because latest was broken; it's fixed now, take me back to latest."
async fn update_one(entry: ToolEntry, force: bool, config: &Config) -> Result<()> {
    let name = entry.binary_name.clone();
    let pin = Manifest::load()?.is_pinned(&entry.repo, entry.binary());
    if let Some(tag) = &pin
        && !force
    {
        print_info(&format!(
            "{name} is pinned to {tag}. \
             Re-run `binto update {name} --force` to update it to the latest release \
             (this clears the pin)."
        ));
        return Ok(());
    }

    print_info(&format!("Checking {name} ({})...", entry.repo));
    let reporter = DownloadReporter::default();
    let ctx = Ctx::new(config, &reporter, &TermPrompter)?;
    let installed = report(run(vec![newer(entry, config)], &ctx).await?)?;

    if let (Some(tag), Some(previous)) = (pin, installed.first().and_then(|d| d.replaced.as_ref()))
    {
        Manifest::set_tag_and_save(&previous.repo, previous.binary(), None)?;
        print_info(&format!("Cleared pin on {name} (was {tag})."));
    }
    Ok(())
}

fn newer(entry: ToolEntry, config: &Config) -> Target {
    Target::update(entry, ReleaseSource::Newer, &config.install_dir)
}

#[derive(serde::Serialize)]
struct CheckResult {
    name: String,
    installed_tag: String,
    latest_tag: Option<String>,
    update_available: bool,
}

pub async fn cmd_check(json: bool, config: &Config) -> Result<()> {
    let state = State::load()?;
    if state.is_empty() {
        if json {
            println!("[]");
        } else {
            print_info("No tools managed by binto.");
        }
        return Ok(());
    }

    // Pinned tools are skipped up front (a pin is a lock), so no request is wasted on them —
    // same policy as `update --all`.
    let manifest = Manifest::load()?;
    let mut targets = Vec::new();
    for (_, entry) in state.iter() {
        if manifest.is_pinned(&entry.repo, entry.binary()).is_some() {
            print_warning(&format!(
                "{} is pinned to {}. Skipping update check.",
                entry.repo, entry.installed_tag
            ));
        } else {
            targets.push(newer(entry.clone(), config));
        }
    }

    // Only looks, never installs: a newer release's ETag is deliberately not stored, or the
    // next check would get a 304 and report "up to date" while still on the old version.
    let ctx = Ctx::new(config, &(), &TermPrompter)?;
    let resolved = resolve_all(targets, &ctx).await?;
    record_checked(&resolved)?;
    let results: Vec<CheckResult> = resolved.into_iter().filter_map(check_result).collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        print_check(&results);
    }

    // Exit 1 if updates available (useful for scripting)
    if results.iter().any(|r| r.update_available) {
        // it's alright because no resource leak
        std::process::exit(1);
    }
    Ok(())
}

fn check_result(Resolved { target, resolution }: Resolved) -> Option<CheckResult> {
    let entry = target.existing()?;
    let (latest_tag, update_available) = match resolution {
        Ok(Resolution::UpToDate { latest }) => (latest, false),
        Ok(Resolution::Release { release, .. }) => (Some(release.tag_name), true),
        Err(e) => {
            print_warning(&format!("Failed to check {}: {e:#}", entry.binary_name));
            return None;
        }
    };
    Some(CheckResult {
        name: entry.binary_name.clone(),
        installed_tag: entry.installed_tag.clone(),
        latest_tag,
        update_available,
    })
}

fn print_check(results: &[CheckResult]) {
    for r in results {
        if r.update_available {
            print_status(&format!(
                "  {} {} → {} (update available)",
                console::style(&r.name).yellow().bold(),
                r.installed_tag,
                r.latest_tag.as_deref().unwrap_or("?")
            ));
        } else {
            print_up_to_date(&r.name, &r.installed_tag);
        }
    }
    if results.iter().any(|r| r.update_available) {
        print_status(&format!(
            "\n{}",
            console::style("Updates available — run `binto update --all`").yellow()
        ));
    }
}
