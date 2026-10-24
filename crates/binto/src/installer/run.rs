//! Runs [`Target`]s through the install stages: resolve the release, pick the asset,
//! download, install. Each tool ends as one [`Outcome`]; a tool that fails doesn't stop the
//! rest, but a Ctrl-C inside a prompt stops everything.

use anyhow::{Context, Result};
use futures_util::{StreamExt, stream};

use crate::config::Config;
use crate::error::is_interrupted;
use crate::github::GithubClient;
use crate::installer::plan::{self, Resolution, Target};
use crate::installer::reporter::{Prompter, Reporter};
use crate::installer::{Downloaded, InstallResult, InstallSpec};
use crate::output::{print_info, print_warning};
use crate::state::{State, ToolEntry};

/// What a command runs with.
pub struct Ctx<'a> {
    pub client: GithubClient,
    pub config: &'a Config,
    pub reporter: &'a dyn Reporter,
    pub prompter: &'a dyn Prompter,
    /// How many requests or downloads run at once.
    pub concurrency: usize,
}

/// How many release lookups and downloads run at once unless `BINTO_CONCURRENT_DOWNLOADS`
/// says otherwise.
const DEFAULT_CONCURRENCY: usize = 8;

/// `BINTO_CONCURRENT_DOWNLOADS`, else [`DEFAULT_CONCURRENCY`]. At least 1.
fn concurrency_from_env() -> usize {
    let var = std::env::var("BINTO_CONCURRENT_DOWNLOADS").ok();
    parse_concurrency(var.as_deref())
}

fn parse_concurrency(var: Option<&str>) -> usize {
    match var.map(|v| v.trim().parse::<usize>()) {
        Some(Ok(n)) => n.max(1),
        Some(Err(_)) => {
            print_warning(&format!(
                "BINTO_CONCURRENT_DOWNLOADS must be a number; using {DEFAULT_CONCURRENCY}"
            ));
            DEFAULT_CONCURRENCY
        }
        None => DEFAULT_CONCURRENCY,
    }
}

impl<'a> Ctx<'a> {
    pub fn new(
        config: &'a Config,
        reporter: &'a dyn Reporter,
        prompter: &'a dyn Prompter,
    ) -> Result<Self> {
        Ok(Self {
            client: GithubClient::from_config(config)?,
            config,
            reporter,
            prompter,
            concurrency: concurrency_from_env(),
        })
    }
}

/// A target with its release looked up.
pub struct Resolved {
    pub target: Target,
    pub resolution: Result<Resolution>,
}

/// A spec with its asset downloaded.
struct Fetched {
    spec: InstallSpec,
    download: Result<Downloaded>,
}

/// A tool that was installed.
pub struct Installed {
    pub result: InstallResult,
    /// The tracked entry it replaced, for an update.
    pub replaced: Option<ToolEntry>,
}

/// What happened to one tool.
pub enum Outcome {
    Installed(Box<Installed>),
    UpToDate {
        name: String,
        tag: String,
    },
    /// Left at `installed` because the manifest pins it to `pin`.
    Pinned {
        name: String,
        installed: String,
        pin: String,
    },
    /// The user said no to taking over a binary already on `$PATH`.
    Declined {
        name: String,
    },
    Failed {
        name: String,
        error: anyhow::Error,
    },
}

impl Outcome {
    pub fn name(&self) -> &str {
        match self {
            Self::Installed(done) => &done.result.tool_entry.binary_name,
            Self::UpToDate { name, .. }
            | Self::Pinned { name, .. }
            | Self::Declined { name }
            | Self::Failed { name, .. } => name,
        }
    }

    fn failed(name: &str, error: anyhow::Error) -> Self {
        Self::Failed {
            name: name.to_string(),
            error,
        }
    }

    /// Order outcomes by tool name, for stable output.
    pub fn sort(outcomes: &mut [Outcome]) {
        outcomes.sort_by(|a, b| a.name().cmp(b.name()));
    }
}

/// Run every target to an outcome, sorted by name.
pub async fn run(targets: Vec<Target>, ctx: &Ctx<'_>) -> Result<Vec<Outcome>> {
    let resolved = resolve_all(targets, ctx).await?;
    record_checked(&resolved)?;

    let mut outcomes = Vec::new();
    let specs = plan_all(resolved, ctx, &mut outcomes)?;
    let fetched = download_all(specs, ctx).await;
    install_all(fetched, ctx, &mut outcomes)?;

    Outcome::sort(&mut outcomes);
    Ok(outcomes)
}

/// Resolve each target's release, sorted by label so prompts and output come in a stable
/// order. Targets that prompt (the release picker) go one at a time first, and a Ctrl-C there
/// stops everything; the rest look up `ctx.concurrency` at a time.
pub async fn resolve_all(targets: Vec<Target>, ctx: &Ctx<'_>) -> Result<Vec<Resolved>> {
    let (prompting, quiet): (Vec<_>, Vec<_>) = targets.into_iter().partition(Target::prompts);

    let mut resolved = Vec::with_capacity(prompting.len() + quiet.len());
    for target in prompting {
        let Resolved { target, resolution } = resolve_one(target, ctx).await;
        match resolution {
            Err(e) if is_interrupted(&e) => return Err(e),
            resolution => resolved.push(Resolved { target, resolution }),
        }
    }
    resolved.extend(
        stream::iter(quiet)
            .map(|target| resolve_one(target, ctx))
            .buffer_unordered(ctx.concurrency)
            .collect::<Vec<_>>()
            .await,
    );
    resolved.sort_by(|a, b| a.target.label().cmp(b.target.label()));
    Ok(resolved)
}

async fn resolve_one(target: Target, ctx: &Ctx<'_>) -> Resolved {
    let resolution = plan::resolve(&ctx.client, &target, ctx.prompter).await;
    Resolved { target, resolution }
}

/// Stamp `last_checked` on every tracked tool whose release was looked up.
pub fn record_checked(resolved: &[Resolved]) -> Result<()> {
    let checked: Vec<&str> = resolved
        .iter()
        .filter(|r| r.resolution.is_ok())
        .filter_map(|r| r.target.existing().map(|e| e.binary_name.as_str()))
        .collect();
    if checked.is_empty() {
        return Ok(());
    }
    State::mutate(|s| checked.iter().for_each(|name| s.touch_checked(name)))
}

/// Turn each resolution into a spec, or an outcome when there's nothing to install.
fn plan_all(
    resolved: Vec<Resolved>,
    ctx: &Ctx<'_>,
    outcomes: &mut Vec<Outcome>,
) -> Result<Vec<InstallSpec>> {
    let mut specs = Vec::new();
    for Resolved { target, resolution } in resolved {
        match plan_one(&target, resolution, ctx) {
            Ok(Planned::Install(spec)) => specs.push(*spec),
            Ok(Planned::Done(outcome)) => outcomes.push(outcome),
            Err(e) if is_interrupted(&e) => return Err(e),
            Err(e) => outcomes.push(Outcome::failed(target.label(), e)),
        }
    }
    Ok(specs)
}

enum Planned {
    Install(Box<InstallSpec>),
    Done(Outcome),
}

fn plan_one(target: &Target, resolution: Result<Resolution>, ctx: &Ctx<'_>) -> Result<Planned> {
    let (release, etag) = match resolution? {
        Resolution::Release { release, etag } => (release, etag),
        Resolution::UpToDate { .. } => {
            let existing = target
                .existing()
                .context("only a tracked tool is up to date")?;
            return Ok(Planned::Done(Outcome::UpToDate {
                name: existing.binary_name.clone(),
                tag: existing.installed_tag.clone(),
            }));
        }
    };
    let spec = plan::spec_for(target, release, etag, ctx.config.prefer_libc, ctx.prompter)?;
    if target.confirm_unmanaged() && !take_over_unmanaged(&spec, ctx.prompter)? {
        return Ok(Planned::Done(Outcome::Declined {
            name: spec.install_name().to_string(),
        }));
    }
    Ok(Planned::Install(Box::new(spec)))
}

/// Whether to go ahead when a binary binto doesn't manage already answers to this name.
fn take_over_unmanaged(spec: &InstallSpec, prompter: &dyn Prompter) -> Result<bool> {
    let name = spec.install_name();
    let Some(existing) = crate::find_on_path(name) else {
        return Ok(true);
    };
    print_warning(&format!(
        "'{name}' is already installed at {}",
        existing.display()
    ));
    if !prompter.interactive() {
        print_info("Installing anyway; binto will manage it going forward.");
        return Ok(true);
    }
    prompter.confirm(
        "Install anyway and let binto manage it going forward?",
        false,
    )
}

/// Download `ctx.concurrency` assets at a time, largest first so the long ones don't start
/// last. Back in name order afterwards, for installing.
async fn download_all(mut specs: Vec<InstallSpec>, ctx: &Ctx<'_>) -> Vec<Fetched> {
    specs.sort_by_key(|spec| std::cmp::Reverse(spec.size()));
    let mut fetched: Vec<Fetched> = stream::iter(specs)
        .map(|spec| async move {
            let download = spec.download(ctx.client.http_client(), ctx.reporter).await;
            Fetched { spec, download }
        })
        .buffer_unordered(ctx.concurrency)
        .collect()
        .await;
    fetched.sort_by(|a, b| a.spec.install_name().cmp(b.spec.install_name()));
    fetched
}

/// Install each download and record it in state right away, so a later failure can't leave
/// an installed binary untracked.
fn install_all(fetched: Vec<Fetched>, ctx: &Ctx<'_>, outcomes: &mut Vec<Outcome>) -> Result<()> {
    for Fetched { spec, download } in fetched {
        let outcome = match download.and_then(|dl| install_one(&spec, dl, ctx.prompter)) {
            Ok(result) => Outcome::Installed(Box::new(Installed {
                result,
                replaced: spec.replaces().cloned(),
            })),
            Err(e) if is_interrupted(&e) => return Err(e),
            Err(e) => Outcome::failed(spec.install_name(), e),
        };
        outcomes.push(outcome);
    }
    Ok(())
}

fn install_one(
    spec: &InstallSpec,
    dl: Downloaded,
    prompter: &dyn Prompter,
) -> Result<InstallResult> {
    let result = spec.install(dl, prompter)?;
    State::mutate(|s| s.upsert(result.tool_entry.clone())).with_context(|| {
        format!(
            "installed {} but failed to record it in state",
            result.installed_path.display()
        )
    })?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrency_defaults_and_never_drops_to_zero() {
        assert_eq!(parse_concurrency(None), DEFAULT_CONCURRENCY);
        assert_eq!(parse_concurrency(Some("3")), 3);
        assert_eq!(parse_concurrency(Some(" 16 ")), 16);
        assert_eq!(parse_concurrency(Some("0")), 1);
        assert_eq!(parse_concurrency(Some("lots")), DEFAULT_CONCURRENCY);
    }
}
