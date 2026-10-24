pub mod binary;
pub mod checksum;
pub mod download;
pub mod extract;
pub mod plan;
pub mod reporter;
pub mod run;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;

use crate::error::BintoError;
use crate::github::types::{Asset, Release};
use crate::installer::extract::ArchiveKind;
use crate::matcher::facts::AssetName;
use crate::matcher::pattern::asset_to_pattern;
use crate::output::{print_info, print_warning};
use crate::state::ToolEntry;
use extract::{BinarySearchResult, find_binary};
use reporter::{Prompter, Reporter};

/// RAII guard that removes temp files on drop.
struct InstallGuard {
    temp_files: Vec<PathBuf>,
    temp_dirs: Vec<PathBuf>,
}

impl InstallGuard {
    fn new() -> Self {
        Self {
            temp_files: vec![],
            temp_dirs: vec![],
        }
    }

    fn track_file(&mut self, path: PathBuf) {
        self.temp_files.push(path);
    }

    fn track_dir(&mut self, path: PathBuf) {
        self.temp_dirs.push(path);
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        for f in &self.temp_files {
            let _ = std::fs::remove_file(f);
        }
        for d in &self.temp_dirs {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

pub struct InstallResult {
    pub tool_entry: ToolEntry,
    pub installed_path: PathBuf,
    /// Which of the repo's binaries this is, when its release ships several. The manifest
    /// key for the tool; `None` for the usual one-binary repo.
    pub variant: Option<String>,
}

// TODO: check if we do too much string operations on repo. It could be its own type then.
/// The repo-derived default name for a tool: the segment after `owner/`. Used both to locate
/// the binary inside an archive and as the installed filename when no `--alias` is given.
pub fn default_binary_name(repo: &str) -> &str {
    repo.split('/').next_back().unwrap_or(repo)
}

/// A verified download of an [`InstallSpec`]'s asset, ready to install. Its temp files are
/// removed when it drops.
pub struct Downloaded {
    asset_path: PathBuf,
    sha256: Option<String>,
    guard: InstallGuard,
}

/// One concrete installation: the release, the asset chosen from it, where it goes and what
/// it is called. Owned, so it can move through the download and install stages on its own.
/// Built by [`plan::spec_for`], the one place install and update differ.
pub struct InstallSpec {
    repo: String,
    release: Release,
    asset: Asset,
    install_dir: PathBuf,
    /// Locates the binary *inside* an archive (exact-match then single-ELF fallback).
    /// Repo- or stem-derived, never the alias.
    find_name: String,
    /// The installed filename and the key the tool is tracked by in state.
    install_name: String,
    /// Which of the repo's binaries this is, when the release ships several.
    variant: Option<String>,
    /// The release's ETag, recorded so the next update check can be conditional.
    etag: Option<String>,
    /// The tracked entry this replaces, for an update.
    replaces: Option<ToolEntry>,
}

impl InstallSpec {
    pub fn install_name(&self) -> &str {
        &self.install_name
    }

    /// The asset's advertised size in bytes.
    pub fn size(&self) -> u64 {
        self.asset.size
    }

    /// The tracked entry this replaces, for an update; `None` for a fresh install.
    pub fn replaces(&self) -> Option<&ToolEntry> {
        self.replaces.as_ref()
    }

    /// Download the asset into the cache and checksum-verify it against the release's
    /// checksum asset, when it publishes one.
    #[tracing::instrument(name = "download", skip_all, fields(name = %self.install_name))]
    pub async fn download(
        &self,
        client: &reqwest::Client,
        reporter: &dyn Reporter,
    ) -> Result<Downloaded> {
        let asset = &self.asset;
        let mut guard = InstallGuard::new();

        // Track before streaming: a Ctrl-C drops this future mid-download, and the guard must
        // remove the partial file.
        let asset_path = self.cache_path(&asset.name);
        guard.track_file(asset_path.clone());
        download::download_to_cache(
            client,
            &asset.browser_download_url,
            &asset_path,
            reporter,
            &self.install_name,
            asset.size,
        )
        .await
        .with_context(|| format!("failed to download {}", asset.name))?;

        let sha256 = match checksum::find_checksum_asset(&asset.name, &self.release.assets) {
            Some(chk_asset) => {
                tracing::debug!("Checksum asset: {:?}", chk_asset);
                Some(checksum::verify_checksum(client, &asset_path, &asset.name, chk_asset).await?)
            }
            None => {
                tracing::debug!("no checksum asset published; recording local sha256");
                checksum::sha256_file(&asset_path).ok()
            }
        };

        Ok(Downloaded {
            asset_path,
            sha256,
            guard,
        })
    }

    /// Extract (or treat as a raw binary), locate the binary, atomic-install it, and
    /// build the resulting `ToolEntry`. Sequential — may ask `prompter` which binary to take.
    #[tracing::instrument(skip_all, fields(repo = %self.repo, install_name = %self.install_name))]
    pub fn install(&self, mut dl: Downloaded, prompter: &dyn Prompter) -> Result<InstallResult> {
        let asset_lower = self.asset.name.to_lowercase();
        let binary_src = match ArchiveKind::from_filename(&asset_lower) {
            Some(archive) => self.extract_binary(archive, &mut dl, prompter)?,
            None => dl.asset_path.clone(),
        };

        let installed_path =
            binary::atomic_install(&binary_src, &self.install_dir, &self.install_name)?;

        // Guard drops with `dl`, removing the temp files (it owns the cache paths, not
        // installed_path).
        Ok(InstallResult {
            tool_entry: self.tool_entry(&installed_path, dl.sha256.clone()),
            installed_path,
            variant: self.variant.clone(),
        })
    }

    /// A cache path of this spec's own. Keyed by the install name (unique per tracked tool)
    /// as well as `name`, so two tools installing same-named assets at once — two binaries
    /// of one archive, or two repos with generic asset names — never share a file.
    fn cache_path(&self, name: &str) -> PathBuf {
        download::cache_dir().join(format!("{}.{name}", self.install_name))
    }

    /// Unpack the archive into the cache and find the binary in it.
    fn extract_binary(
        &self,
        archive: ArchiveKind,
        dl: &mut Downloaded,
        prompter: &dyn Prompter,
    ) -> Result<PathBuf> {
        let extract_dir = self.cache_path(&format!("{}-extract", self.asset.name));
        dl.guard.track_dir(extract_dir.clone());

        print_info("Extracting archive...");
        extract::extract_archive(archive, &dl.asset_path, &extract_dir)?;

        match find_binary(&extract_dir, &self.find_name)? {
            BinarySearchResult::Found(p) => {
                tracing::debug!(path = %p.display(), "located binary in archive");
                Ok(p)
            }
            BinarySearchResult::Multiple(candidates) => self.pick_binary(candidates, prompter),
            BinarySearchResult::NotFound => Err(BintoError::BinaryNotFoundInArchive.into()),
        }
    }

    /// The state entry recording this installation.
    fn tool_entry(&self, installed_path: &Path, sha256: Option<String>) -> ToolEntry {
        ToolEntry {
            repo: self.repo.clone(),
            installed_tag: self.release.tag_name.clone(),
            install_path: installed_path.to_path_buf(),
            binary_name: self.install_name.clone(),
            // Which binary of the repo this is, so a repo shipping several can tell its
            // entries apart even when one carries an `--alias`.
            stem: AssetName::new(&self.asset.name).stem(Some(&self.release.tag_name)),
            asset_pattern: asset_to_pattern(&self.asset.name, &self.release.tag_name),
            installed_sha256: sha256,
            etag: self.etag.clone(),
            last_checked: Some(Utc::now()),
            published_at: Some(self.release.published_at),
        }
    }

    /// Choose among several executables found in the archive. Without a person to ask, take
    /// the one named like the tool (`fd-helper` over `debug` for `fd`), else the first, and
    /// say which.
    fn pick_binary(
        &self,
        mut candidates: Vec<PathBuf>,
        prompter: &dyn Prompter,
    ) -> Result<PathBuf> {
        let idx = if prompter.interactive() {
            let names: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
            prompter
                .select("Multiple binaries found — pick one", &names)
                .context("failed to show binary picker")?
        } else {
            candidates
                .iter()
                .position(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(&self.find_name))
                })
                .unwrap_or(0)
        };
        anyhow::ensure!(idx < candidates.len(), "binary pick {idx} is out of range");
        let chosen = candidates.swap_remove(idx);
        if !prompter.interactive() {
            print_warning(&format!(
                "The archive holds several executables; installing {}",
                chosen.display()
            ));
        }
        Ok(chosen)
    }
}
