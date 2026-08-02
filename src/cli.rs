use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "binto",
    about = "GitHub Release manager — user-land binary package manager"
)]
#[command(version)]
pub struct Cli {
    /// Increase logging verbosity on the terminal (repeatable: -v debug, -vv trace)
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Only show warnings and errors on the terminal
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Fetch releases for a repo, pick one, and install the matching asset
    #[command(visible_alias = "i")]
    Install {
        /// GitHub repository as owner/repo or a github.com URL
        repo: String,
        /// Pin to a specific release tag instead of picking interactively
        #[arg(short = 't', long)]
        tag: Option<String>,
        /// Install the binary under this name instead of the repo-derived default
        #[arg(short = 'a', long)]
        alias: Option<String>,
        /// Install into this directory instead of the configured install_dir
        #[arg(long, value_name = "PATH")]
        to: Option<PathBuf>,
        /// Include pre-releases in the release list
        #[arg(long)]
        prerelease: bool,
        /// Non-interactive: auto-pick the asset, skip prompts, and use the latest release if no -t
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Show all managed tools with installed version and update status
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Update one or all managed tools to the latest release
    Update {
        /// Name of the tool to update (omit for --all)
        name: Option<String>,
        /// Update all managed tools
        #[arg(long)]
        all: bool,
        /// Force-update a pinned tool to the latest release, clearing its pin
        #[arg(short = 'f', long)]
        force: bool,
    },

    /// Check for updates and print a summary (good for timer/scripting use)
    Check {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Register an already-installed binary under binto management
    Adopt {
        /// Path to the existing binary
        path: String,
        /// GitHub repository as owner/repo or a github.com URL
        repo: String,
    },

    /// Uninstall a binary and remove it from binto state
    Remove {
        /// Name of the tool to remove
        name: String,
        /// Skip the confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Install everything in the manifest that is missing from local state
    Sync {
        /// Also remove managed tools that are not listed in the manifest
        #[arg(long)]
        prune: bool,
        /// Skip the prune confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Remove binto's download cache (`~/.cache/binto`)
    Clean,

    /// Uninstall binto: always removes its state, optionally binaries/manifest/config/logs
    ///
    /// binto never deletes its own executable as a special step. The intended way to remove
    /// the binto binary is to let binto manage itself, so it gets removed as one of the
    /// installed binaries. There are two clean ways out:
    ///
    ///   * Get rid of binto entirely: select "Remove installed binaries" (and whatever else
    ///     you want gone). If binto is self-managed, this deletes the binto binary along with
    ///     the tools it installed.
    ///
    ///   * Keep your tools, drop binto's management: leave "Remove installed binaries"
    ///     unselected and remove the rest. Your binaries (binto included) stay on PATH and
    ///     become yours to manage by hand; binto simply stops tracking them.
    ///
    /// NOTE: Nuking data directory WILL REMOVE any binary installed in the default installation folder.
    ///
    /// Deleting the binto binary on its own is deliberately not offered and not recommended.
    /// It would just orphan binto's data.
    #[command(verbatim_doc_comment)]
    Uninstall,
    /// Run the asset-matching algorithm against a release JSON file and print the verdict.
    ///
    /// For the matching test harness — not intended for end users. Reads a release JSON
    /// (a GitHub release response, or `{"tag": "...", "assets": [...]}`) from a file or stdin,
    /// runs the matcher, and writes a single JSON verdict to stdout. The decision-point trace
    /// goes to stderr (as JSON when `BINTO_LOG_FORMAT=json` and `-v`/`-vv` are set). Exit code:
    /// 0 = auto-selected, 42 = needs interaction, 43 = no compatible asset.
    #[command(hide = true, verbatim_doc_comment)]
    Match {
        /// Repository label (owner/repo) — recorded in the verdict and trace, not fetched.
        repo: String,
        /// Release JSON file. Reads stdin when omitted or `-`.
        #[arg(short = 'f', long, value_name = "FILE")]
        file: Option<PathBuf>,
        /// Override the release tag (default: the file's `tag_name`/`tag`, else "unknown").
        #[arg(short = 't', long)]
        tag: Option<String>,
        /// Target architecture to match for (default: the host's `uname -m`).
        #[arg(short = 'a', long)]
        arch: Option<String>,
        /// Preferred libc: `gnu` or `musl` (default: the configured `prefer_libc`).
        #[arg(long)]
        libc: Option<String>,
    },

    /// Generate and optionally enable a systemd user timer for automatic update checks
    SetupTimer,

    /// Stop, disable, and remove the binto systemd timer
    DisableTimer,
}
