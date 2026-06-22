mod adopt;
mod clean;
mod cli;
mod config;
mod error;
mod github;
mod install;
mod installer;
mod logging;
mod manifest;
mod matcher;
mod output;
mod picker;
mod state;
mod sync;
mod timer;
mod ui_format;
mod update;

use anyhow::{Context, Result};
use clap::Parser;
use console::style;

use cli::{Cli, Commands};

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
use config::Config;
use output::{print_error, print_info, print_status, print_success, print_warning};
use state::{State, ToolEntry};

use crate::manifest::Manifest;

#[tokio::main]
async fn main() {
    install_ctrlc_handler();

    // Parse before any async/spawn so logging is live for the whole run. The worker guard must
    // outlive every log call — hold it here until the process exits, or the file log truncates.
    let cli = Cli::parse();
    let _log_guard = logging::init(cli.verbose, cli.quiet);

    if let Err(e) = run(cli).await {
        print_error(&e);
        std::process::exit(1);
    }
}

/// Restore the terminal cursor on Ctrl-C.
///
/// Interactive prompts (dialoguer's release/asset pickers) hide the cursor while open and
/// only restore it on Enter/Escape. console reads Ctrl-C in raw mode and re-raises SIGINT to
/// us, which the default handler turns into an immediate exit — *before* dialoguer can show
/// the cursor again, leaving the terminal with an invisible cursor until `reset`. This
/// handler runs on its own thread (so terminal I/O + exit are safe), shows the cursor, and
/// exits with the conventional 130 (128 + SIGINT).
fn install_ctrlc_handler() {
    let _ = ctrlc::set_handler(|| {
        let _ = console::Term::stderr().show_cursor();
        let _ = console::Term::stdout().show_cursor();
        std::process::exit(130);
    });
}

async fn run(cli: Cli) -> Result<()> {
    let config = Config::load()?;

    // Stale-check banner: warn if any tool hasn't been checked recently
    maybe_print_stale_banner(&config);

    match cli.command {
        Commands::Install {
            repo,
            tag,
            alias,
            to,
            prerelease,
        } => {
            let repo = parse_repo(&repo)?;
            install::cmd_install(&repo, tag, alias, to, prerelease, &config).await?;
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
                "{stale_count} tool(s) haven't been checked recently — run `binto check`"
            ))
            .yellow()
        ));
    }
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
        if manifest.is_pinned(&entry.repo).is_some() {
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
/// touch the manifest — callers batch those (and `sync --prune` deliberately leaves the
/// manifest alone). Returns the removed entry so callers can sync the manifest by repo.
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
    // TODO: add a funny condition for where binto tries to remove itself
    let mut state = State::load()?;

    let entry = state.require(name)?.clone();

    if !yes {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Remove {} ({}) from {} ?",
                name,
                entry.installed_tag,
                entry.install_path.display()
            ))
            .default(false)
            .interact()?;
        if !confirmed {
            print_info("Aborted.");
            return Ok(());
        }
    }

    remove_tool(&mut state, name)?;
    state.save()?;

    // Keep the declarative manifest in sync: drop the row for this tool's repo so a later
    // `binto sync` won't reinstall it. State is keyed by binary name, the manifest by repo.
    // Format-preserving write: comments and unrelated entries are left untouched.
    manifest::Manifest::remove_and_save(&entry.repo)?;

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
