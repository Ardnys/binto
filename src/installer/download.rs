use std::path::PathBuf;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use indicatif::ProgressStyle;
use tracing::Span;
use tracing_indicatif::span_ext::IndicatifSpanExt;

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".cache"))
        .join("ghr")
}

/// The byte-progress bar template, rendered on a download span's progress bar.
fn byte_progress_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{msg:.bold}  {bar:45.green/black.dim}  {bytes:>10} / {total_bytes:<10}  {bytes_per_sec:>12}  eta {eta}",
    )
    .unwrap()
    .progress_chars("━──")
}

/// Build the `download` span that renders a byte-progress bar for one asset. The
/// `indicatif.pb_show` field opts the span in to a bar (the `IndicatifFilter` hides bars for
/// every other span). `name` labels the bar and is the per-tool log context. `.instrument()`
/// this span onto the download future so the bar follows it across `.await` points — never hold
/// an `.enter()` guard across an await.
pub fn download_span(name: &str, total: u64) -> Span {
    let span = tracing::info_span!(
        "download",
        name = %name,
        "indicatif.pb_show" = tracing::field::Empty,
    );
    span.pb_set_style(&byte_progress_style());
    span.pb_set_message(name);
    // Seed the bar with the asset's advertised size; `download_to_cache` refines it from the
    // response's content-length.
    if total > 0 {
        span.pb_set_length(total);
    }
    span
}

/// Stream an asset to the cache, updating the *current* span's progress bar as bytes arrive.
/// Must run inside a [`download_span`] (the concurrent loops `.instrument()` it; single installs
/// go through `InstallSpec::run`).
pub async fn download_to_cache(
    client: &reqwest::Client,
    url: &str,
    filename: &str,
) -> Result<PathBuf> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create cache dir {}", dir.display()))?;

    let dest = dir.join(filename);

    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;

    if !resp.status().is_success() {
        anyhow::bail!("download failed with status {}", resp.status());
    }

    let span = Span::current();
    if let Some(len) = resp.content_length() {
        span.pb_set_length(len);
    }
    tracing::debug!(filename, "streaming asset to cache");

    let mut file = tokio::fs::File::create(&dest)
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
        span.pb_set_position(downloaded);
    }

    tracing::debug!(bytes = downloaded, "download complete");
    Ok(dest)
}
