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

    /// Generate and optionally enable a systemd user timer for automatic update checks
    SetupTimer,

    /// Stop, disable, and remove the binto systemd timer
    DisableTimer,
}
