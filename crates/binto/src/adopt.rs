use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;

use crate::config::Config;
use crate::installer::checksum::sha256_file;
use crate::installer::extract::is_executable;
use crate::manifest::Manifest;
use crate::output::{print_success, print_warning};
use crate::state::{State, ToolEntry};

pub async fn cmd_adopt(path: String, repo: String, _config: &Config) -> Result<()> {
    let bin_path = PathBuf::from(&path);

    if !bin_path.exists() {
        anyhow::bail!("path does not exist: {}", bin_path.display());
    }

    if !bin_path.is_file() {
        anyhow::bail!("path is not a file: {}", bin_path.display());
    }

    if !is_executable(&bin_path) {
        anyhow::bail!("{} is not executable", bin_path.display());
    }

    let binary_name = bin_path
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| "could not determine binary name from path")?
        .to_string();

    let state = State::load()?;

    if state.contains(&binary_name) {
        print_warning(&format!(
            "{binary_name} is already managed by binto. Use `binto update {binary_name}` instead."
        ));
        return Ok(());
    }

    let sha256 = sha256_file(&bin_path).ok();

    let entry = ToolEntry {
        repo: repo.clone(),
        installed_tag: "unknown".to_string(),
        install_path: bin_path.canonicalize().unwrap_or(bin_path.clone()),
        binary_name: binary_name.clone(),
        asset_pattern: String::new(),
        installed_sha256: sha256,
        etag: None,
        last_checked: Some(Utc::now()),
        published_at: None,
    };

    State::mutate(|s| s.upsert(entry))?;

    // Record the adopted tool in the portable manifest (unpinned) so `binto sync` on another
    // machine reinstalls it from its GitHub releases.
    Manifest::set_tag_and_save(&repo, None)?;

    print_success(&format!(
        "Adopted {binary_name} ({repo}). Run `binto update {binary_name}` to detect the current version."
    ));

    Ok(())
}
