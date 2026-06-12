mod adopt;
mod cli;
mod config;
mod error;
mod github;
mod installer;
mod matcher;
mod output;
mod state;
mod timer;
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
use github::GithubClient;
use matcher::{MatchOutput, match_asset, score::detect_arch};
use output::{print_error, print_info, print_success, print_warning};
use state::State;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        print_error(&e);
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;

    // Stale-check banner: warn if any tool hasn't been checked recently
    maybe_print_stale_banner(&config);

    match cli.command {
        Commands::Install { repo, prerelease } => {
            let repo = parse_repo(&repo)?;
            cmd_install(&repo, prerelease, &config).await?;
        }
        Commands::List { json } => {
            cmd_list(json, &config)?;
        }
        Commands::Update { name, all } => {
            update::cmd_update(name, all, &config).await?;
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
    if state.tools.is_empty() {
        return;
    }

    let threshold = chrono::Duration::hours(config.check_interval_hours as i64);
    let now = chrono::Utc::now();

    let stale_count = state
        .tools
        .values()
        .filter(|e| e.last_checked.map(|t| now - t > threshold).unwrap_or(true))
        .count();

    if stale_count > 0 {
        println!(
            "{}",
            style(format!(
                "{stale_count} tool(s) haven't been checked recently — run `ghr check`"
            ))
            .yellow()
        );
    }
}

async fn cmd_install(repo: &str, include_prerelease: bool, config: &Config) -> Result<()> {
    // Check for an already-managed tool with the same repo before doing any network I/O.
    let mut state = State::load()?;
    let binary_name = repo.split('/').next_back().unwrap_or(repo);
    if let Some(existing) = state.tools.get(binary_name)
        && existing.repo == repo
    {
        anyhow::bail!(
            "'{binary_name}' is already managed by ghr ({}). \
                 Run `ghr update {binary_name}` to upgrade it.",
            existing.installed_tag
        );
    }

    // Check if the binary already exists somewhere on $PATH outside of ghr.
    if let Some(existing_path) = find_on_path(binary_name) {
        print_warning(&format!(
            "'{binary_name}' is already installed at {}",
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

    // Fetch release list for interactive picker
    let mut releases = client.list_releases(repo).await?;

    if !include_prerelease && !config.include_prereleases {
        releases.retain(|r| !r.prerelease && !r.draft);
    } else {
        releases.retain(|r| !r.draft);
    }

    if releases.is_empty() {
        anyhow::bail!("no releases found for {repo}");
    }

    // Interactive release picker
    let release_labels: Vec<String> = releases
        .iter()
        .map(|r| format!("{} ({})", r.tag_name, r.published_at.format("%Y-%m-%d")))
        .collect();

    let release_idx = dialoguer::FuzzySelect::new()
        .with_prompt("Pick a release")
        .items(&release_labels)
        .default(0)
        .interact()?;

    let release = &releases[release_idx];
    println!("Selected: {} — {}", release.tag_name, release.html_url);

    // Match asset
    let user_arch = detect_arch();
    let all_assets = release.assets.clone();

    let match_output = match_asset(
        all_assets.clone(),
        &user_arch,
        None,
        repo,
        &release.tag_name,
    )?;

    let selected = match match_output {
        MatchOutput::AutoSelected(s) => {
            print_info(&format!(
                "Auto-selected asset: {} with {} arch: {}",
                s.asset.name, s.score.arch_match, &user_arch
            ));
            s
        }
        MatchOutput::NeedsInteraction(candidates) => {
            let names: Vec<String> = candidates.iter().map(|c| c.asset.name.clone()).collect();
            let idx = dialoguer::Select::new()
                .with_prompt("Pick an asset")
                .items(&names)
                .default(0)
                .interact()?;
            candidates.into_iter().nth(idx).unwrap()
        }
    };

    let result = installer::install_asset(
        client.http_client(),
        repo,
        release,
        &selected.asset,
        binary_name,
        &config.install_dir,
        &all_assets,
    )
    .await?;

    state
        .tools
        .insert(binary_name.to_string(), result.tool_entry);
    state.save()?;

    print_success(&format!(
        "Installed {} {} → {}",
        binary_name,
        release.tag_name,
        result.installed_path.display()
    ));

    // Warn if install_dir is not on PATH
    if let Ok(path_var) = std::env::var("PATH") {
        let on_path = path_var
            .split(':')
            .any(|p| std::path::Path::new(p) == config.install_dir);
        if !on_path {
            print_warning(&format!(
                "{} is not on your PATH. Add it: export PATH=\"{}:$PATH\"",
                config.install_dir.display(),
                config.install_dir.display()
            ));
        }
    }

    Ok(())
}

fn cmd_list(json: bool, _config: &Config) -> Result<()> {
    let state = State::load()?;

    if state.tools.is_empty() {
        print_info("No tools managed by ghr. Run `ghr install <owner/repo>` to get started.");
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&state.tools)?);
        return Ok(());
    }

    // Table header
    println!(
        "{:<20} {:<15} {:<30} {}",
        style("NAME").bold(),
        style("VERSION").bold(),
        style("REPO").bold(),
        style("LAST CHECKED").bold()
    );
    println!("{}", "-".repeat(80));

    for (name, entry) in &state.tools {
        let last_checked = entry
            .last_checked
            .map(|t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "never".to_string());

        println!(
            "{:<20} {:<15} {:<30} {}",
            style(name).green(),
            entry.installed_tag,
            entry.repo,
            last_checked
        );
    }

    Ok(())
}

fn cmd_remove(name: &str, yes: bool, _config: &Config) -> Result<()> {
    let mut state = State::load()?;

    let entry = state
        .tools
        .get(name)
        .ok_or_else(|| crate::error::GhrError::UnknownTool {
            name: name.to_string(),
        })?
        .clone();

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

    // Remove binary
    if entry.install_path.exists() {
        std::fs::remove_file(&entry.install_path)
            .with_context(|| format!("failed to remove {}", entry.install_path.display()))?;
    } else {
        print_warning(&format!(
            "Binary not found at {} — removing from state only.",
            entry.install_path.display()
        ));
    }

    state.tools.shift_remove(name);
    state.save()?;

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
