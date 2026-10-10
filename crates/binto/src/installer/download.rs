use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use futures_util::StreamExt;

use crate::installer::reporter::{DownloadProgress, Reporter};

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".cache"))
        .join("binto")
}

/// Stream an asset to `dest`, reporting each chunk to `reporter` under `name`. The bar's
/// length is the response's content-length, else `size_hint` (the asset's advertised size).
pub async fn download_to_cache(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    reporter: &dyn Reporter,
    name: &str,
    size_hint: u64,
) -> Result<()> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create cache dir {}", dir.display()))?;

    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    if !resp.status().is_success() {
        anyhow::bail!("download failed with status {}", resp.status());
    }

    let size = resp.content_length().or(Some(size_hint).filter(|&s| s > 0));
    let progress = DownloadProgress::start(reporter, name, size);
    tracing::debug!(dest = %dest.display(), "streaming asset to cache");

    let mut file = tokio::fs::File::create(dest)
        .await
        .with_context(|| format!("failed to create {}", dest.display()))?;

    let mut stream = resp.bytes_stream();
    let mut downloaded = 0u64;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("error reading download stream")?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .context("error writing to cache file")?;
        downloaded += chunk.len() as u64;
        progress.inc(chunk.len() as u64);
    }

    tracing::debug!(bytes = downloaded, "download complete");
    Ok(())
}
