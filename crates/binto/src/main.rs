mod adopt;
mod clean;
mod cli;
mod config;
mod error;
mod github;
mod install;
mod installer;
mod lock;
mod logging;
mod manifest;
mod matcher;
mod output;
mod picker;
mod reporter;
mod state;
mod sync;
mod timer;
mod ui_format;
mod uninstall;
mod update;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use binto_contract::{Candidate, MatchInput, MatchVerdict, Outcome};
use clap::Parser;
use console::style;
use matcher::rank::{RankedAsset, SelectionNote};
use matcher::{MatchOutput, match_asset};

use cli::{Cli, Commands};
use config::Config;
use config::Libc;
use error::{BintoError, is_interrupted};
use installer::checksum::find_checksum_asset;
use output::{print_error, print_info, print_status, print_success, print_warning};
use state::{State, ToolEntry};

use crate::{manifest::Manifest, matcher::facts::detect_arch};

/// Return the first path on $PATH where `name` exists as a file, if any.
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).find_map(|dir| {
        let candidate = dir.join(name);
        if candidate.is_file() {
            Some(candidate)
        } else {
            None
        }
    })
}

// TODO: this is vendor dependent
// TODO: move this out of main.rs
/// Accept either "owner/repo" or any github.com URL and return "owner/repo".
fn parse_repo(input: &str) -> Result<String> {
    let s = input.trim().trim_end_matches('/');
    // Fast-path: already in owner/repo form (no scheme, exactly one slash)
    if !s.contains("://") && !s.starts_with("github.com") {
        if s.matches('/').count() == 1 {
            return Ok(s.to_string());
        }
        anyhow::bail!("'{input}' is not a valid owner/repo or GitHub URL");
    }
    // Strip scheme and optional www/github.com prefix
    let path = s
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("github.com/");
    // path is now "owner/repo[/...]"; keep only first two segments
    let mut parts = path.splitn(3, '/');
    let owner = parts.next().filter(|p| !p.is_empty());
    let repo = parts.next().filter(|p| !p.is_empty());
    match (owner, repo) {
        (Some(o), Some(r)) => Ok(format!("{o}/{r}")),
        _ => anyhow::bail!("'{input}' is not a valid GitHub URL"),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let _log_guard = logging::init(cli.verbose, cli.quiet);

    let runtime = tokio::runtime::Runtime::new().expect("failed to start the tokio runtime");
    // Ctrl-C drops the `run` future instead of exiting the process, so destructors (temp-file
    // guards, locks) run. `biased` polls `ctrl_c` first, registering the handler before `run`
    // does any work.
    let outcome = runtime.block_on(async {
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => None,
            res = run(cli) => Some(res),
        }
    });
    // Dropping the runtime drops the tasks still in flight (e.g. `update --all` downloads),
    // running their guards too.
    drop(runtime);

    match outcome {
        Some(Ok(())) => ExitCode::SUCCESS,
        Some(Err(e)) if !is_interrupted(&e) => {
            print_error(&e);
            ExitCode::FAILURE
        }
        // Interactive prompts (dialoguer) hide the cursor and only restore it on Enter/Escape.
        // A Ctrl-C inside one makes console return an `Interrupted` error (after raising SIGINT,
        // which the tokio handler absorbs), so `run` usually finishes before `ctrl_c` is seen.
        _ => {
            let _ = console::Term::stderr().show_cursor();
            let _ = console::Term::stdout().show_cursor();
            ExitCode::from(130)
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let config = Config::load()?;

    // Stale-check banner: warn if any tool hasn't been checked recently. Skipped for `match`,
    // whose stderr must stay a clean decision trace for the test harness.
    if !matches!(cli.command, Commands::Match { .. }) {
        maybe_print_stale_banner(&config);
    }

    // TODO: it would be nice to have command in a `commands/` folder to keep `src/` tidy
    match cli.command {
        Commands::Install {
            repo,
            tag,
            alias,
            to,
            prerelease,
            yes,
        } => {
            let repo = parse_repo(&repo)?;
            install::cmd_install(&repo, tag, alias, to, prerelease, yes, &config).await?;
        }
        Commands::List { json } => {
            cmd_list(json, &config)?;
        }
        Commands::Update { name, all, force } => {
            update::cmd_update(name, all, force, &config).await?;
        }
        Commands::Check { json } => {
            update::cmd_check(json, &config).await?;
        }
        Commands::Adopt { path, repo } => {
            let repo = parse_repo(&repo)?;
            adopt::cmd_adopt(path, repo, &config).await?;
        }
        Commands::Remove { name, yes } => {
            cmd_remove(&name, yes, &config)?;
        }
        Commands::Sync { prune, yes } => {
            sync::cmd_sync(&config, prune, yes).await?;
        }
        Commands::Clean => {
            clean::cmd_clean()?;
        }
        Commands::Uninstall => {
            uninstall::cmd_uninstall()?;
        }
        Commands::Match {
            repo,
            file,
            tag,
            arch,
            libc,
        } => {
            cmd_match_asset(&repo, file, tag, arch, libc, &config)?;
        }
        Commands::SetupTimer => {
            timer::cmd_setup_timer()?;
        }
        Commands::DisableTimer => {
            timer::cmd_disable_timer()?;
        }
    }

    Ok(())
}

fn maybe_print_stale_banner(config: &Config) {
    let Ok(state) = State::load() else { return };
    if state.is_empty() {
        return;
    }

    let threshold = chrono::Duration::hours(config.check_interval_hours as i64);
    let now = chrono::Utc::now();

    let stale_count = state
        .iter()
        .filter(|(_, e)| e.last_checked.map(|t| now - t > threshold).unwrap_or(true))
        .count();

    if stale_count > 0 {
        print_status(&format!(
            "{}",
            style(format!(
                "{stale_count} tool(s) haven't been checked recently. run `binto check`"
            ))
            .yellow()
        ));
    }
}

/// Build a verdict [`Candidate`] from a ranked asset.
fn candidate_of(r: &RankedAsset, notes: &[SelectionNote]) -> Candidate {
    let [(_, arch), (_, os), (_, libc), (_, format)] = r.labels();
    Candidate {
        name: r.name().to_string(),
        stem: r.stem().to_string(),
        tiers: binto_contract::Tiers {
            arch: arch.to_string(),
            os: os.to_string(),
            libc: libc.to_string(),
            format: format.to_string(),
        },
        notes: notes.iter().map(note_of).collect(),
    }
}

fn note_of(note: &SelectionNote) -> binto_contract::SelectionNote {
    match note {
        SelectionNote::Fallback {
            dimension,
            wanted,
            got,
        } => binto_contract::SelectionNote::Fallback {
            dimension: dimension.to_string(),
            wanted: wanted.to_string(),
            got: got.to_string(),
        },
        SelectionNote::Unspecified { dimension } => binto_contract::SelectionNote::Unspecified {
            dimension: dimension.to_string(),
        },
    }
}

/// Run the matcher against a release read from a file (or stdin) and print a JSON verdict to
/// stdout. The per-decision trace is emitted on stderr via `tracing` (JSON when
/// `BINTO_LOG_FORMAT=json`). Exit code encodes the outcome for the harness: 0 auto-selected,
/// 42 needs interaction, 43 no compatible asset.
fn cmd_match_asset(
    repo: &str,
    file: Option<PathBuf>,
    tag_override: Option<String>,
    arch_override: Option<String>,
    libc_override: Option<String>,
    config: &Config,
) -> Result<()> {
    let raw = match file {
        Some(p) if p.as_os_str() != "-" => std::fs::read_to_string(&p)
            .with_context(|| format!("failed to read release file {}", p.display()))?,
        _ => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read release JSON from stdin")?;
            buf
        }
    };

    let input: MatchInput = serde_json::from_str(&raw).context(
        "failed to parse release JSON (expected a GitHub release object or {\"assets\": [...]})",
    )?;

    let tag = tag_override
        .or(input.tag)
        .unwrap_or_else(|| "unknown".to_string());
    let arch = arch_override.unwrap_or_else(detect_arch);
    let prefer_libc = match libc_override.as_deref() {
        None => config.prefer_libc,
        Some("gnu") => Libc::Gnu,
        Some("musl") => Libc::Musl,
        Some(other) => anyhow::bail!("invalid --libc '{other}' (expected 'gnu' or 'musl')"),
    };
    let libc = match prefer_libc {
        Libc::Gnu => "gnu",
        Libc::Musl => "musl",
    };

    let assets = input.assets;
    let result = match_asset(assets.clone(), &arch, None, repo, &tag, prefer_libc);

    let verdict = match result {
        Ok(MatchOutput::AutoSelected(s)) => {
            let checksum = find_checksum_asset(&s.asset().name, &assets).map(|a| a.name.clone());
            let candidate = candidate_of(&s.ranked, &s.notes);
            MatchVerdict {
                repo: repo.to_string(),
                tag,
                arch,
                libc: libc.to_string(),
                outcome: Outcome::AutoSelected,
                selected: Some(candidate.clone()),
                checksum,
                candidates: vec![candidate],
            }
        }
        // Only the leader carries notes: they describe what the whole tie group has in
        // common, which is usually why it tied.
        Ok(MatchOutput::NeedsInteraction { ranked, notes }) => MatchVerdict {
            repo: repo.to_string(),
            tag,
            arch,
            libc: libc.to_string(),
            outcome: Outcome::NeedsInteraction,
            selected: None,
            checksum: None,
            candidates: ranked
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let notes = if i == 0 { notes.as_slice() } else { &[] };
                    candidate_of(r, notes)
                })
                .collect(),
        },
        // A missing/incompatible asset is a normal harness outcome, not a hard error — report it
        // as a verdict. Any other error (there are none today) propagates.
        Err(e) if e.downcast_ref::<BintoError>().is_some() => MatchVerdict {
            repo: repo.to_string(),
            tag,
            arch,
            libc: libc.to_string(),
            outcome: Outcome::NoMatch,
            selected: None,
            checksum: None,
            candidates: vec![],
        },
        Err(e) => return Err(e),
    };

    // Write the verdict to stdout and flush explicitly: `process::exit` skips destructors, so a
    // block-buffered pipe would otherwise drop it.
    let mut out = std::io::stdout();
    writeln!(out, "{}", serde_json::to_string_pretty(&verdict)?)
        .and_then(|_| out.flush())
        .context("failed to write verdict to stdout")?;

    std::process::exit(verdict.outcome.exit_code());
}

fn cmd_list(json: bool, _config: &Config) -> Result<()> {
    let state = State::load()?;
    let manifest = Manifest::load()?;

    if state.is_empty() {
        print_info("No tools managed by binto. Run `binto install <owner/repo>` to get started.");
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&state.tools)?);
        return Ok(());
    }

    // TODO: it would be nice if the table was more intelligent
    // Table header
    println!(
        "{:<20} {:<15} {:<30} {}",
        style("NAME").bold(),
        style("VERSION").bold(),
        style("REPO").bold(),
        style("LAST CHECKED").bold()
    );
    println!("{}", "-".repeat(80));

    for (name, entry) in state.iter() {
        let last_checked = entry
            .last_checked
            .map(|t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "never".to_string());

        let mut tag = entry.installed_tag.clone();

        // put an asterisk on pinned release tags
        if manifest.is_pinned(&entry.repo, entry.binary()).is_some() {
            tag.push('*');
        }

        println!(
            "{:<20} {:<15} {:<30} {}",
            style(name).green(),
            tag,
            entry.repo,
            last_checked
        );
    }

    Ok(())
}

/// Delete a managed tool's binary from disk and drop it from `state`. Does NOT save state or
/// touch the manifest — callers run it inside `State::try_mutate` (and `sync --prune`
/// deliberately leaves the manifest alone). Returns the removed entry so callers can sync the manifest by repo.
/// Shared by `binto remove` and `binto sync --prune`.
pub fn remove_tool(state: &mut State, name: &str) -> Result<ToolEntry> {
    let entry = state.require(name)?.clone();

    if entry.install_path.exists() {
        std::fs::remove_file(&entry.install_path)
            .with_context(|| format!("failed to remove {}", entry.install_path.display()))?;
    } else {
        print_warning(&format!(
            "Binary not found at {} — removing from state only.",
            entry.install_path.display()
        ));
    }

    state.remove(name);
    Ok(entry)
}

fn cmd_remove(name: &str, yes: bool, _config: &Config) -> Result<()> {
    let state = State::load()?;

    let entry = state.require(name)?.clone();

    if entry.repo == "Ardnys/binto" {
        print_warning(
            "You are trying to remove binto, with binto. To properly remove binto, see `binto uninstall --help`",
        );
        anyhow::bail!("Aborted.");
    }

    let confirmed = reporter::prompter(yes).confirm(
        &format!(
            "Remove {} ({}) from {} ?",
            name,
            entry.installed_tag,
            entry.install_path.display()
        ),
        false,
    )?;
    if !confirmed {
        print_info("Aborted.");
        return Ok(());
    }

    // Delete the binary + drop the entry under the global lock, re-reading fresh state so a
    // concurrent mutation isn't lost. `remove_tool` re-`require`s, erroring cleanly if the tool
    // vanished between the prompt and here.
    let entry = State::try_mutate(|s| remove_tool(s, name))?;

    // Keep the declarative manifest in sync: drop the row for this tool's repo so a later
    // `binto sync` won't reinstall it. State is keyed by binary name, the manifest by repo.
    // Format-preserving write: comments and unrelated entries are left untouched.
    manifest::Manifest::remove_and_save(&entry.repo, entry.binary())?;

    print_success(&format!("Removed {name}."));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_repo_owner_slash_repo() {
        assert_eq!(
            parse_repo("BurntSushi/ripgrep").unwrap(),
            "BurntSushi/ripgrep"
        );
    }

    #[test]
    fn parse_repo_https_url() {
        assert_eq!(
            parse_repo("https://github.com/BurntSushi/ripgrep").unwrap(),
            "BurntSushi/ripgrep"
        );
    }

    #[test]
    fn parse_repo_url_with_trailing_slash() {
        assert_eq!(
            parse_repo("https://github.com/BurntSushi/ripgrep/").unwrap(),
            "BurntSushi/ripgrep"
        );
    }

    #[test]
    fn parse_repo_url_with_subpath() {
        assert_eq!(
            parse_repo("https://github.com/cli/cli/releases/latest").unwrap(),
            "cli/cli"
        );
    }

    #[test]
    fn parse_repo_http_url() {
        assert_eq!(
            parse_repo("http://github.com/sharkdp/bat").unwrap(),
            "sharkdp/bat"
        );
    }

    #[test]
    fn parse_repo_without_scheme() {
        assert_eq!(parse_repo("github.com/sharkdp/bat").unwrap(), "sharkdp/bat");
    }

    #[test]
    fn parse_repo_invalid_bare_string() {
        assert!(parse_repo("notarepo").is_err());
    }

    #[test]
    fn parse_repo_invalid_url_no_repo() {
        assert!(parse_repo("https://github.com/onlyowner").is_err());
    }
}
