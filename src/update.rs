use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use tokio::task::JoinSet;
use tracing::Instrument;

use crate::config::Config;
use crate::github::api::{ApiResponse, ConditionalResult, GithubClient};
use crate::github::types::{Asset, Release};
use crate::installer::download::download_span;
use crate::installer::{Downloaded, InstallSpec};
use crate::manifest::Manifest;
use crate::output::{print_info, print_status, print_success, print_warning};
use crate::picker::select_asset;
use crate::state::{State, ToolEntry};

/// Carries a tool's state across the concurrent updater's phase boundary: the download phase
/// (fanned out) produces a `Downloaded` keyed by `name`, which the sequential install phase
/// pairs back up here to build an `InstallSpec`.
struct PendingUpdate {
    name: String,
    entry: ToolEntry,
    release: Release,
    asset: Asset,
    install_dir: PathBuf,
    new_etag: Option<String>,
}

fn print_up_to_date(name: &str, tag: &str) {
    print_status(&format!(
        "  {} {tag} (up to date)",
        console::style(name).green().bold(),
    ));
}

/// Concurrent update for all tools: fan-out API checks → sequential asset selection
/// → concurrent downloads → sequential extract+install.
pub async fn cmd_update_concurrent(config: &Config) -> Result<()> {
    let mut state = State::load()?;

    if state.is_empty() {
        print_info("No tools managed by binto. Run `binto install <owner/repo>` to get started.");
        return Ok(());
    }

    let manifest = Manifest::load()?;
    let token = GithubClient::resolve_token(config.github_token.clone());
    let client = GithubClient::new(token)?;
    let user_arch = crate::matcher::score::detect_arch();

    // Phase A: concurrent API checks. Pinned tools are skipped up front so we don't waste
    // a request — a pin is a lock, so `update --all` deliberately leaves them alone.
    let snapshot: Vec<(String, ToolEntry)> =
        state.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

    let mut api_set: JoinSet<(String, ToolEntry, Result<ConditionalResult<Release>>)> =
        JoinSet::new();

    for (name, entry) in snapshot {
        if let Some(tag) = manifest.is_pinned(&entry.repo) {
            print_status(&format!(
                "  {} {} (pinned {tag})",
                console::style(&name).blue().bold(),
                entry.installed_tag,
            ));
            continue;
        }
        let client = client.clone();
        let repo = entry.repo.clone();
        let etag = entry.etag.clone();
        api_set.spawn(async move {
            let result = client.get_latest_release(&repo, etag.as_deref()).await;
            (name, entry, result)
        });
    }

    let mut api_results: Vec<(String, ToolEntry, Result<ConditionalResult<Release>>)> = Vec::new();
    while let Some(res) = api_set.join_next().await {
        api_results.push(res?);
    }
    api_results.sort_by(|a, b| a.0.cmp(&b.0));

    // Phase B: sequential asset selection — collect tools that need updating
    let mut pending: Vec<PendingUpdate> = Vec::new();

    for (name, entry, result) in api_results {
        match result {
            Ok(ConditionalResult::NotModified) => {
                print_up_to_date(&name, &entry.installed_tag);
                state.touch_checked(&name);
            }
            Ok(ConditionalResult::Changed(ApiResponse {
                data: release,
                etag: new_etag,
            })) => {
                state.touch_checked(&name);

                if !entry.is_behind(release.published_at) {
                    print_up_to_date(&name, &entry.installed_tag);
                    continue;
                }

                print_status(&format!(
                    "  {} {} → {}",
                    console::style(&name).yellow().bold(),
                    entry.installed_tag,
                    release.tag_name
                ));

                let asset = select_asset(
                    &release,
                    &user_arch,
                    Some(&entry.asset_pattern),
                    &entry.repo,
                    &format!("Pick an asset for {name}"),
                )?;
                let install_dir = entry.install_dir(&config.install_dir).to_path_buf();

                pending.push(PendingUpdate {
                    name,
                    entry,
                    release,
                    asset,
                    install_dir,
                    new_etag,
                });
            }
            Err(e) => {
                print_warning(&format!("Failed to check {name}: {e:#}"));
            }
        }
    }

    if pending.is_empty() {
        state.save()?;
        return Ok(());
    }

    // Phase C: concurrent downloads. Each task runs inside its own `download` span, which
    // renders the byte-progress bar (via tracing-indicatif) and tags the task's log events.
    let http = client.http_client().clone();
    let mut pending_map: HashMap<String, PendingUpdate> = HashMap::new();
    let mut dl_set: JoinSet<(String, Result<Downloaded>)> = JoinSet::new();

    for p in pending {
        let task_name = p.name.clone();
        let http = http.clone();
        let asset = p.asset.clone();
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
        pending_map.insert(p.name.clone(), p);
    }

    let mut downloads: Vec<(String, Downloaded)> = Vec::new();
    while let Some(res) = dl_set.join_next().await {
        let (name, dl_result) = res?;
        match dl_result {
            Ok(dl) => downloads.push((name, dl)),
            Err(e) => {
                pending_map.remove(&name);
                print_warning(&format!("Failed to download {name}: {e:#}"));
            }
        }
    }

    if downloads.is_empty() {
        state.save()?;
        return Ok(());
    }

    // Phase D: sequential extract + install (handles interactive binary picker safely)
    for (name, dl) in downloads {
        let Some(p) = pending_map.remove(&name) else {
            continue;
        };
        // The archive still ships the upstream-named binary; the builder locates it by the
        // repo-derived name, but (re)installs under the tracked name — which may be an `--alias`.
        let spec = InstallSpec::builder(&p.entry.repo, &p.release, &p.asset)
            .install_dir(&p.install_dir)
            .install_name(&p.entry.binary_name)
            .build();
        match spec.install(dl) {
            Ok(ir) => {
                state.upsert(ir.tool_entry.with_etag(p.new_etag));
                print_success(&format!(
                    "{name}: {} → {}",
                    p.entry.installed_tag, p.release.tag_name
                ));
            }
            Err(e) => {
                print_warning(&format!("Failed to install {name}: {e:#}"));
            }
        }
    }

    state.save()?;
    Ok(())
}

/// Sequential update for a single named tool. `--all` is routed to the concurrent path.
pub async fn cmd_update(
    name: Option<String>,
    all: bool,
    force: bool,
    config: &Config,
) -> Result<()> {
    if all {
        if force {
            print_info(
                "--force has no effect with --all; pinned tools stay locked. \
                 Name a tool to force-update it.",
            );
        }
        return cmd_update_concurrent(config).await;
    }

    let mut state = State::load()?;

    if state.is_empty() {
        print_info("No tools managed by binto. Run `binto install <owner/repo>` to get started.");
        return Ok(());
    }

    let tool_name = name.ok_or_else(|| anyhow::anyhow!("specify a tool name or pass --all"))?;
    let entry = state.require(&tool_name)?.clone();

    // A pinned tag is a lock: refuse to update unless `--force` is given. With `--force` the
    // lock is released (the pin is cleared) and the tool is updated to the latest release —
    // the common case being "I pinned an older version because latest was broken; it's fixed
    // now, take me back to latest."
    let manifest = Manifest::load()?;
    let pinned_tag = manifest.is_pinned(&entry.repo).map(str::to_string);
    if let Some(tag) = pinned_tag {
        if !force {
            print_info(&format!(
                "{tool_name} is pinned to {tag}. \
                 Re-run `binto update {tool_name} --force` to update it to the latest release \
                 (this clears the pin)."
            ));
            return Ok(());
        }

        Manifest::set_tag_and_save(&entry.repo, None)?;
        print_info(&format!(
            "Cleared pin on {tool_name} (was {tag}); updating to the latest release."
        ));
    }

    let token = GithubClient::resolve_token(config.github_token.clone());
    let client = GithubClient::new(token)?;

    print_info(&format!("Checking {} ({})...", tool_name, entry.repo));

    let result = client
        .get_latest_release(&entry.repo, entry.etag.as_deref())
        .await;

    match result {
        Ok(ConditionalResult::NotModified) => {
            print_success(&format!(
                "{tool_name} is already up to date (304 Not Modified)."
            ));
            state.touch_checked(&tool_name);
        }
        Ok(ConditionalResult::Changed(ApiResponse {
            data: release,
            etag: new_etag,
        })) => {
            state.touch_checked(&tool_name);

            if !entry.is_behind(release.published_at) {
                print_success(&format!(
                    "{tool_name} is already up to date ({}).",
                    entry.installed_tag
                ));
                state.save()?;
                return Ok(());
            }

            print_status(&format!(
                "  Update available: {} → {}",
                entry.installed_tag, release.tag_name
            ));

            let user_arch = crate::matcher::score::detect_arch();
            let asset = select_asset(
                &release,
                &user_arch,
                Some(&entry.asset_pattern),
                &entry.repo,
                "Pick an asset",
            )?;
            let install_dir = entry.install_dir(&config.install_dir).to_path_buf();

            // Locate the binary in the archive by its upstream name; reinstall under the
            // tracked name (which may be an `--alias`).
            let result = InstallSpec::builder(&entry.repo, &release, &asset)
                .install_dir(&install_dir)
                .install_name(&entry.binary_name)
                .build()
                .run(client.http_client())
                .await?;

            state.upsert(result.tool_entry.with_etag(new_etag));
            print_success(&format!(
                "{tool_name} updated to {} → {}",
                entry.installed_tag, release.tag_name
            ));
        }
        Err(e) => {
            print_warning(&format!("Failed to check {tool_name}: {e:#}"));
        }
    }

    state.save()?;
    Ok(())
}

pub async fn cmd_check(json: bool, config: &Config) -> Result<()> {
    let mut state = State::load()?;
    let manifest = Manifest::load()?;

    if state.is_empty() {
        if json {
            println!("[]");
        } else {
            print_info("No tools managed by binto.");
        }
        return Ok(());
    }

    let token = GithubClient::resolve_token(config.github_token.clone());
    let client = GithubClient::new(token)?;

    #[derive(serde::Serialize)]
    struct CheckResult {
        name: String,
        installed_tag: String,
        latest_tag: Option<String>,
        update_available: bool,
    }

    let snapshot: Vec<(String, ToolEntry)> =
        state.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

    let mut results: Vec<CheckResult> = Vec::new();
    let mut any_updates = false;

    // Fan out the API checks concurrently. Pinned tools are skipped up front (a pin is a
    // lock), so no request is wasted on them — same policy as `update --all`.
    let mut api_set: JoinSet<(String, ToolEntry, Result<ConditionalResult<Release>>)> =
        JoinSet::new();

    for (name, entry) in snapshot {
        if manifest.is_pinned(&entry.repo).is_some() {
            print_warning(&format!(
                "{} is pinned to {}. Skipping update check.",
                entry.repo, entry.installed_tag
            ));
            continue;
        }
        let client = client.clone();
        let repo = entry.repo.clone();
        let etag = entry.etag.clone();
        api_set.spawn(async move {
            let result = client.get_latest_release(&repo, etag.as_deref()).await;
            (name, entry, result)
        });
    }

    let mut api_results: Vec<(String, ToolEntry, Result<ConditionalResult<Release>>)> = Vec::new();
    while let Some(res) = api_set.join_next().await {
        api_results.push(res?);
    }
    api_results.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, entry, result) in api_results {
        state.touch_checked(&name);

        match result {
            Ok(ConditionalResult::NotModified) => {
                results.push(CheckResult {
                    name,
                    installed_tag: entry.installed_tag.clone(),
                    latest_tag: None,
                    update_available: false,
                });
            }
            Ok(ConditionalResult::Changed(resp)) => {
                let release = resp.data;
                // NOTE: deliberately do NOT persist the ETag here. Storing the latest
                // release's ETag without installing it would make the next check return
                // 304 and wrongly report "up to date" while still on the old version.
                let update_available = entry.is_behind(release.published_at);
                any_updates |= update_available;

                results.push(CheckResult {
                    name,
                    installed_tag: entry.installed_tag.clone(),
                    latest_tag: Some(release.tag_name),
                    update_available,
                });
            }
            Err(e) => {
                print_warning(&format!("Failed to check {name}: {e:#}"));
            }
        }
    }

    state.save()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        for r in &results {
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

        if any_updates {
            print_status(&format!(
                "\n{}",
                console::style("Updates available — run `binto update --all`").yellow()
            ));
        }
    }

    // Exit 1 if updates available (useful for scripting)
    if any_updates {
        std::process::exit(1);
    }

    Ok(())
}
