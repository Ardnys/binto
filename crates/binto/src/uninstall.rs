use std::{fmt::Display, io::ErrorKind, path::Path};

use anyhow::{Context, Result};
use tracing::{debug, debug_span};

use crate::{
    config::Config,
    lock::lock_path,
    logging::log_dir,
    manifest::Manifest,
    output::{print_info, print_warning},
    remove_tool,
    state::State,
};

struct UninstallationOptions<'a> {
    option: &'a str,
    func: fn() -> Result<()>,
}

impl<'a> UninstallationOptions<'a> {
    fn new(option: &'a str, func: fn() -> Result<()>) -> Self {
        UninstallationOptions { option, func }
    }
    fn run(&self) -> Result<()> {
        (self.func)()
    }
}

// Display implements ToString trait, which is needed for dialoguer::MultiSelect.items
impl<'a> Display for UninstallationOptions<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.option)
    }
}

fn rm_file(path: &Path) -> Result<()> {
    let _ = debug_span!("rm_file");
    match std::fs::remove_file(path) {
        Ok(()) => {
            debug!("Removed successfully: {}", path.display());
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            debug!("File not found: {}", path.display());
            Ok(())
        }
        Err(e) => {
            debug!("Failed to remove file: {}", path.display());
            Err(e).with_context(|| format!("failed to remove {}", path.display()))
        }
    }
}

fn rm_dir(path: &Path) -> Result<()> {
    let _ = debug_span!("rm_dir");
    match std::fs::remove_dir_all(path) {
        Ok(()) => {
            debug!("Removed successfully: {}", path.display());
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            debug!("Directory not found: {}", path.display());
            Ok(())
        }
        Err(e) => {
            debug!(
                "Failed to remove directory: {} because {}",
                path.display(),
                e
            );
            Err(e).with_context(|| format!("failed to remove {}", path.display()))
        }
    }
}

/// Deletes all binaries in the `state.toml`
fn remove_bins() -> Result<()> {
    State::mutate(|s| {
        let names: Vec<String> = s.iter().map(|(n, _)| n.clone()).collect();
        for name in &names {
            let entry = remove_tool(s, name)?;
            print_info(&format!("Removed: {}", entry.binary_name));
        }
        Ok::<_, anyhow::Error>(())
    })?
}
/// Deletes `manifest.toml` at `Manifest::manifest_path()`
fn remove_manifest() -> Result<()> {
    let manifest_path = Manifest::manifest_path();
    rm_file(&manifest_path)?;
    print_info("Removed manifest file");
    Ok(())
}
/// Deletes `config.toml` at `Config::config_path()`
fn remove_config() -> Result<()> {
    let config_path = Config::config_path();
    rm_file(&config_path)?;
    print_info("Removed config file");
    Ok(())
}
/// deletes the log directory at `log_dir`
fn remove_logs() -> Result<()> {
    let log_path = log_dir();
    rm_dir(&log_path)?;
    print_info("Removed logs");
    Ok(())
}
/// always removed during uninstallation
fn remove_always() -> Result<()> {
    let state_path = State::state_path();
    let lock_path = lock_path();
    rm_file(&state_path)?;
    rm_file(&lock_path)?;
    Ok(())
}

/// Remove the data directory altogether
fn nuke_data() -> Result<()> {
    if let Some(p) = State::state_path().parent() {
        rm_dir(p)?;
        print_info(&format!("Deleted {}", p.display()));
    }

    Ok(())
}

/// Uninstallation command
/// Optionally removes binaries, logs, manifest file and config file
/// Always removes state.toml once its done removing other stuff
/// When nothing is selected, aborts without removing anything
/// User is free to select what to remove. Not removing everything
/// could leave the program in corrupted state.
pub fn cmd_uninstall() -> Result<()> {
    print_warning("If you choose to keep the binaries, note that binto will no longer track them");

    let options = vec![
        UninstallationOptions::new("Remove installed binaries", remove_bins),
        UninstallationOptions::new("Remove logs", remove_logs),
        UninstallationOptions::new(
            "Nuke data directory altogether (binaries, logs, state)",
            nuke_data,
        ),
        UninstallationOptions::new("Remove manifest file", remove_manifest),
        UninstallationOptions::new("Remove config file", remove_config),
    ];

    let selection = dialoguer::MultiSelect::new()
        .with_prompt("Select uninstallation options")
        .items(&options)
        .interact()?;

    if selection.is_empty() {
        print_info("Nothing selected. Aborting...");
        return Ok(());
    }

    for i in selection {
        let opt = &options[i];
        let _ = opt.run();
    }

    // remove state after removing every other thing
    remove_always()?;

    Ok(())
}
