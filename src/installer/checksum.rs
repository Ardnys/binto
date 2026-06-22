use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::error::GhrError;
use crate::github::types::Asset;

/// Find the checksum sidecar asset for `target_name` in the full asset list.
pub fn find_checksum_asset<'a>(target_name: &str, all_assets: &'a [Asset]) -> Option<&'a Asset> {
    let lower_target = target_name.to_lowercase();

    all_assets.iter().find(|a| {
        let lower = a.name.to_lowercase();
        // Exact match like "tool_linux_amd64.tar.gz.sha256"
        if lower == format!("{}.sha256", lower_target)
            || lower == format!("{}.sha512", lower_target)
        {
            return true;
        }
        // Generic checksum files
        matches!(
            lower.as_str(),
            "checksums.txt"
                | "sha256sums"
                | "sha256sums.txt"
                | "sha512sums"
                | "sha512sums.txt"
                | "sha256sum"
                | "sha256sum.txt"
                | "checksums.sha256"
                | "checksums"
                | "checksum.txt"
        )
    })
}

/// Parse a checksums file and find the hash for `filename`.
/// Supports the standard `<hash>  <filename>` format (one or two spaces).
pub fn parse_checksums(content: &str, filename: &str) -> Option<String> {
    let lower_filename = filename.to_lowercase();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() < 2 {
            continue;
        }
        let hash = parts[0].trim();
        let name = parts[1].trim().trim_start_matches('*');
        // Match on just the basename
        let basename = Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(name)
            .to_lowercase();
        if basename == lower_filename {
            return Some(hash.to_string());
        }
    }
    None
}

/// Compute the SHA-256 hex digest of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(hex::encode(digest))
}

/// Download checksum file, parse it, and verify the downloaded asset.
#[tracing::instrument(skip_all, fields(asset = %asset_name), err(level = "debug"))]
pub async fn verify_checksum(
    client: &reqwest::Client,
    asset_path: &Path,
    asset_name: &str,
    checksum_url: &str,
) -> Result<String> {
    let resp = client
        .get(checksum_url)
        .send()
        .await
        .context("failed to download checksums file")?;

    if !resp.status().is_success() {
        anyhow::bail!("checksums download failed: {}", resp.status());
    }

    let content = resp.text().await.context("failed to read checksums file")?;
    let expected = parse_checksums(&content, asset_name)
        .with_context(|| format!("could not find checksum for '{asset_name}' in checksums file"))?;

    let got = sha256_file(asset_path)?;
    tracing::debug!(%expected, %got, "comparing checksums");

    if got != expected {
        return Err(GhrError::ChecksumMismatch {
            filename: asset_name.to_string(),
            expected,
            got,
        }
        .into());
    }

    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_checksums_file() {
        let content = "\
abc123  tool_linux_amd64.tar.gz\n\
def456  tool_darwin_arm64.tar.gz\n\
";
        assert_eq!(
            parse_checksums(content, "tool_linux_amd64.tar.gz"),
            Some("abc123".to_string())
        );
        assert_eq!(parse_checksums(content, "nonexistent.tar.gz"), None);
    }

    #[test]
    fn parses_asterisk_prefix() {
        let content = "abc123 *tool_linux_amd64.tar.gz\n";
        assert_eq!(
            parse_checksums(content, "tool_linux_amd64.tar.gz"),
            Some("abc123".to_string())
        );
    }
}
