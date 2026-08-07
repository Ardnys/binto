pub mod binary;
pub mod checksum;
pub mod download;
pub mod extract;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use tracing::Instrument;

use crate::error::BintoError;
use crate::github::types::{Asset, Release};
use crate::matcher::pattern::asset_to_pattern;
use crate::output::print_info;
use crate::state::ToolEntry;
use extract::{BinarySearchResult, find_binary};

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
}

// TODO: check if we do too much string operations on repo. It could be its own type then.
/// The repo-derived default name for a tool: the segment after `owner/`. Used both to locate
/// the binary inside an archive and as the installed filename when no `--alias` is given.
pub fn default_binary_name(repo: &str) -> &str {
    repo.split('/').next_back().unwrap_or(repo)
}

pub struct Downloaded {
    pub asset_path: PathBuf,
    pub sha256: Option<String>,
    guard: InstallGuard,
}

impl Downloaded {
    /// Download the asset into the cache and checksum-verify it. Concurrency-safe —
    /// the `update --all` fan-out clones the asset into each task and calls this directly,
    /// since a borrowing `InstallSpec` method can't satisfy the `'static` bound on spawned
    /// futures. Sequential callers go through `InstallSpec::download`.
    pub async fn fetch(
        client: &reqwest::Client,
        asset: &Asset,
        all_assets: &[Asset],
    ) -> Result<Downloaded> {
        let mut guard = InstallGuard::new();

        let asset_path =
            download::download_to_cache(client, &asset.browser_download_url, &asset.name)
                .await
                .with_context(|| format!("failed to download {}", asset.name))?;
        guard.track_file(asset_path.clone());

        let installed_sha256 =
            if let Some(chk_asset) = checksum::find_checksum_asset(&asset.name, all_assets) {
                tracing::debug!("Checksum asset: {:?}", chk_asset);
                match checksum::verify_checksum(client, &asset_path, &asset.name, chk_asset).await {
                    Ok(hash) => Some(hash),
                    Err(e) => return Err(e),
                }
            } else {
                tracing::debug!("no checksum asset published; recording local sha256");
                checksum::sha256_file(&asset_path).ok()
            };

        Ok(Downloaded {
            asset_path,
            sha256: installed_sha256,
            guard,
        })
    }
}

/// A fully-resolved installation: a concrete release + asset, the directory to install into,
/// and the resolved name split. Bundles everything the download → install pipeline needs so
/// callers pass no loose parameters. Build via [`InstallSpec::builder`].
pub struct InstallSpec<'a> {
    repo: &'a str,
    release: &'a Release,
    asset: &'a Asset,
    install_dir: &'a Path,
    /// Locates the binary *inside* an archive (exact-match then single-ELF fallback).
    /// Repo-derived, never the alias.
    find_name: String,
    /// The installed filename and the key the tool is tracked by in state — the `--alias`
    /// (or a managed tool's existing `binary_name`), else the repo-derived default.
    install_name: String,
}

/// Builder for [`InstallSpec`]. `install_dir` is required; `install_name` defaults to the
/// repo-derived name when not set.
pub struct InstallSpecBuilder<'a> {
    repo: &'a str,
    release: &'a Release,
    asset: &'a Asset,
    install_dir: Option<&'a Path>,
    install_name: Option<String>,
}

impl<'a> InstallSpec<'a> {
    pub fn builder(
        repo: &'a str,
        release: &'a Release,
        asset: &'a Asset,
    ) -> InstallSpecBuilder<'a> {
        InstallSpecBuilder {
            repo,
            release,
            asset,
            install_dir: None,
            install_name: None,
        }
    }
}

impl<'a> InstallSpecBuilder<'a> {
    pub fn install_dir(mut self, dir: &'a Path) -> Self {
        self.install_dir = Some(dir);
        self
    }

    /// Override the installed name (alias, or a managed tool's tracked `binary_name`).
    pub fn install_name(mut self, name: impl Into<String>) -> Self {
        self.install_name = Some(name.into());
        self
    }

    /// Resolve the name split and finalize the spec. Panics if `install_dir` was never set.
    pub fn build(self) -> InstallSpec<'a> {
        let find_name = default_binary_name(self.repo);
        let install_name = self.install_name.unwrap_or_else(|| find_name.to_string());
        InstallSpec {
            repo: self.repo,
            release: self.release,
            asset: self.asset,
            install_dir: self
                .install_dir
                .expect("InstallSpecBuilder::install_dir must be set before build"),
            find_name: find_name.to_string(),
            install_name,
        }
    }
}

impl InstallSpec<'_> {
    /// Download + verify. Thin wrapper over [`Downloaded::fetch`] for sequential callers; the
    /// concurrent updater calls `Downloaded::fetch` directly. Run this inside a
    /// [`download::download_span`] so the byte-progress bar renders (see [`InstallSpec::run`]).
    pub async fn download(&self, client: &reqwest::Client) -> Result<Downloaded> {
        Downloaded::fetch(client, self.asset, &self.release.assets).await
    }

    /// Extract (or treat as a raw binary), locate the binary, atomic-install it, and
    /// build the resulting `ToolEntry`. Sequential — may show the interactive binary picker.
    #[tracing::instrument(skip_all, fields(repo = %self.repo, install_name = %self.install_name))]
    pub fn install(&self, mut dl: Downloaded) -> Result<InstallResult> {
        // Detect archive by filename extension first; fall back to content_type for assets
        // with no extension.
        let asset_lower = self.asset.name.to_lowercase();
        let ct = self.asset.content_type.to_lowercase();
        let is_archive = asset_lower.ends_with(".tar.gz")
            || asset_lower.ends_with(".tgz")
            || asset_lower.ends_with(".tar.xz")
            || asset_lower.ends_with(".tar.bz2")
            || asset_lower.ends_with(".zip")
            || ct.contains("gzip")
            || ct.contains("x-tar")
            || ct.contains("x-xz")
            || ct.contains("x-bzip2")
            || ct == "application/zip";

        let binary_src = if is_archive {
            let extract_dir = download::cache_dir().join(format!("{}-extract", self.asset.name));
            dl.guard.track_dir(extract_dir.clone());

            print_info("Extracting archive...");
            extract::extract_archive(&dl.asset_path, &extract_dir)?;

            // Locate binary inside the extracted tree
            match find_binary(&extract_dir, &self.find_name)? {
                BinarySearchResult::Found(p) => {
                    tracing::debug!(path = %p.display(), "located binary in archive");
                    p
                }
                BinarySearchResult::Multiple(candidates) => {
                    let names: Vec<String> =
                        candidates.iter().map(|p| p.display().to_string()).collect();
                    let selection = dialoguer::Select::new()
                        .with_prompt("Multiple binaries found — pick one")
                        .items(&names)
                        .default(0)
                        .interact()
                        .context("failed to show binary picker")?;
                    candidates.into_iter().nth(selection).unwrap()
                }
                BinarySearchResult::NotFound => {
                    return Err(BintoError::BinaryNotFoundInArchive.into());
                }
            }
        } else {
            dl.asset_path.clone()
        };

        // chmod + atomic install
        let installed_path =
            binary::atomic_install(&binary_src, self.install_dir, &self.install_name)?;

        let asset_pattern = asset_to_pattern(&self.asset.name, &self.release.tag_name);

        let tool_entry = ToolEntry {
            repo: self.repo.to_string(),
            installed_tag: self.release.tag_name.clone(),
            install_path: installed_path.clone(),
            binary_name: self.install_name.clone(),
            asset_pattern,
            installed_sha256: dl.sha256.clone(),
            etag: None,
            last_checked: Some(Utc::now()),
            published_at: Some(self.release.published_at),
        };

        // Guard drops here. Temp files are removed (guard owns asset_path clone, not
        // installed_path).
        Ok(InstallResult {
            tool_entry,
            installed_path,
        })
    }

    /// Both phases back-to-back, for the simple single-tool callers (install / sync / single
    /// update). The concurrent updater interleaves the phases by hand instead. Owns its own
    /// download span so the byte-progress bar renders for the single-tool case.
    pub async fn run(&self, client: &reqwest::Client) -> Result<InstallResult> {
        let span = download::download_span(&self.install_name, self.asset.size);
        let dl = self.download(client).instrument(span).await?;
        self.install(dl)
    }
}
