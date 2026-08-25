use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};

use crate::error::BintoError;
use crate::github::types::Asset;

/// A checksum algorithm we can verify against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChecksumAlgo {
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

// TODO: write this with macros
impl ChecksumAlgo {
    /// Infer the algorithm from the length of a hex-encoded digest, or `None` if no SHA-2 width
    /// matches.
    fn from_hex_len(len: usize) -> Option<Self> {
        match len {
            56 => Some(Self::Sha224),
            64 => Some(Self::Sha256),
            96 => Some(Self::Sha384),
            128 => Some(Self::Sha512),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Sha224 => "sha224",
            Self::Sha256 => "sha256",
            Self::Sha384 => "sha384",
            Self::Sha512 => "sha512",
        }
    }

    /// Hex digest of `bytes` under this algorithm.
    fn digest_hex(self, bytes: &[u8]) -> String {
        match self {
            Self::Sha224 => hex::encode(Sha224::digest(bytes)),
            Self::Sha256 => hex::encode(Sha256::digest(bytes)),
            Self::Sha384 => hex::encode(Sha384::digest(bytes)),
            Self::Sha512 => hex::encode(Sha512::digest(bytes)),
        }
    }
}

/// Whether `s` is a hex string whose length matches a supported digest width.
fn digest_algo(s: &str) -> Option<ChecksumAlgo> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    ChecksumAlgo::from_hex_len(s.len())
}

/// Extensions that mark a per-asset checksum *sidecar* (`<asset>.<ext>`). The actual algorithm is
/// still read from the digest length, so this list only needs to recognize that the file is a
/// checksum, not which kind.
const SIDECAR_EXTS: &[&str] = &[
    "sha256", "sha512", "sha384", "sha224", "sha2", "sha", "digest", "checksum", "sum",
];
// TODO: there's also sigstore.json
//

/// Find the checksum asset that covers `target_name` in the full asset list.
///
/// A per-asset sidecar (`tool.tar.gz.sha512`) is preferred over a repo-wide list, since it holds
/// only this asset's hash and so can't be matched to the wrong line.
pub fn find_checksum_asset<'a>(target_name: &str, all_assets: &'a [Asset]) -> Option<&'a Asset> {
    let lower_target = target_name.to_lowercase();

    if let Some(sidecar) = all_assets.iter().find(|a| {
        a.name
            .to_lowercase()
            .strip_prefix(&lower_target)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|ext| SIDECAR_EXTS.contains(&ext))
    }) {
        tracing::debug!(
            asset = target_name,
            checksum_asset = %sidecar.name,
            kind = "sidecar",
            "found checksum asset"
        );
        return Some(sidecar);
    }

    let generic = all_assets
        .iter()
        .find(|a| is_generic_checksums_file(&a.name));
    match generic {
        Some(a) => tracing::debug!(
            asset = target_name,
            checksum_asset = %a.name,
            kind = "generic",
            "found checksum asset"
        ),
        None => tracing::debug!(
            asset = target_name,
            asset_count = all_assets.len(),
            "no checksum asset found"
        ),
    }
    generic
}

//  gh's checksum file is named gh_2.95.0_checksums.txt, which `is_generic_checksums_file` doesn't match
/// Recognize a repo-wide checksums file: `checksums.txt`, `SHA256SUMS`, `sha512sum.txt`,
/// `checksums.sha256`, and similar.
fn is_generic_checksums_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    let stem = lower.strip_suffix(".txt").unwrap_or(&lower);

    stem == "checksums"
        || stem == "checksum"
        || stem.starts_with("checksums")
        || stem.starts_with("checksum")
        || stem.ends_with("checksums")
        || stem.ends_with("checksum")
        || (stem.starts_with("sha") && (stem.ends_with("sum") || stem.ends_with("sums")))
}

/// Extract the expected hash for `filename` from a checksum file's contents. Returns the hash
/// lowercased.
///
/// Handles both layouts seen in the wild:
///   * a multi-line `<hash>  <filename>` list (`checksums.txt`, `SHA256SUMS`, …); the filename may
///     itself contain spaces, and a leading `*` (gnu coreutils binary mode) is ignored.
///   * a sidecar holding just the bare hash (`tool.tar.gz.sha512` → `deadbeef…`), optionally
///     followed by the filename.
pub fn parse_checksums(content: &str, filename: &str) -> Option<String> {
    let lower_filename = filename.to_lowercase();
    let mut lines_scanned = 0usize;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        lines_scanned += 1;

        // The hash is the first whitespace-delimited token; everything after it is the filename
        // (which may contain spaces).
        let (hash, rest) = match line.split_once(char::is_whitespace) {
            Some((h, r)) => (h, r.trim_start().trim_start_matches('*')),
            // A lone token: a bare-hash sidecar with no accompanying filename.
            None => {
                if digest_algo(line).is_some() {
                    tracing::debug!(
                        filename,
                        hash_len = line.len(),
                        entry = "bare_sidecar",
                        "found checksum entry"
                    );
                    return Some(line.to_lowercase());
                }
                continue;
            }
        };

        let basename = Path::new(rest)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(rest)
            .to_lowercase();
        if basename == lower_filename {
            tracing::debug!(
                filename,
                hash_len = hash.len(),
                entry = "list",
                "found checksum entry"
            );
            return Some(hash.to_lowercase());
        }
    }

    tracing::debug!(filename, lines_scanned, "no checksum entry for file");
    None
}

/// Compute the SHA-256 hex digest of a file. Used to fingerprint binaries that ship without a
/// published checksum (e.g. adopted binaries).
pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

/// Download the checksum file, parse out the expected hash for `asset_name`, then verify the
/// downloaded asset against it. The hashing algorithm is selected from the digest length, so any
/// SHA-2 variant (sha224/256/384/512) a project publishes works without configuration.
///
/// Returns the verified digest on success.
#[tracing::instrument(skip_all, fields(asset = %asset_name), err(level = "debug"))]
pub async fn verify_checksum(
    client: &reqwest::Client,
    asset_path: &Path,
    asset_name: &str,
    chk_asset: &Asset,
) -> Result<String> {
    let resp = client
        .get(&chk_asset.browser_download_url)
        .send()
        .await
        .context("failed to download checksums file")?;

    if !resp.status().is_success() {
        anyhow::bail!("checksums download failed: {}", resp.status());
    }

    let content = resp.text().await.context("failed to read checksums file")?;
    let expected = parse_checksums(&content, asset_name).with_context(|| {
        format!(
            "could not find a checksum for '{asset_name}' in {}",
            chk_asset.name
        )
    })?;

    // Choose the algorithm from the digest length, not the file name.
    let algo = ChecksumAlgo::from_hex_len(expected.len()).with_context(|| {
        format!(
            "unrecognized checksum for '{asset_name}': {}-char digest in {} is not a known SHA-2 length",
            expected.len(),
            chk_asset.name
        )
    })?;

    let bytes = std::fs::read(asset_path)
        .with_context(|| format!("failed to read {}", asset_path.display()))?;
    let got = algo.digest_hex(&bytes);

    tracing::debug!(algo = algo.label(), %expected, %got, "comparing checksums");

    if got != expected {
        return Err(BintoError::ChecksumMismatch {
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

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
            size: 0,
            content_type: String::new(),
        }
    }

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

    #[test]
    fn parses_filename_with_spaces() {
        let content = "abc123  my tool linux.tar.gz\n";
        assert_eq!(
            parse_checksums(content, "my tool linux.tar.gz"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn uppercase_hash_is_normalized() {
        let content = "ABCDEF  tool.tar.gz\n";
        assert_eq!(
            parse_checksums(content, "tool.tar.gz"),
            Some("abcdef".to_string())
        );
    }

    #[test]
    fn parses_bare_hash_sidecar() {
        // A `tool.tar.gz.sha512` sidecar that holds only the digest, with no filename.
        let sha512 = "a".repeat(128);
        assert_eq!(
            parse_checksums(&format!("{sha512}\n"), "tool.tar.gz"),
            Some(sha512.clone())
        );
        // Bare hash followed by the filename also works.
        assert_eq!(
            parse_checksums(&format!("{sha512}  tool.tar.gz\n"), "tool.tar.gz"),
            Some(sha512)
        );
    }

    #[test]
    fn algo_inferred_from_digest_length() {
        assert_eq!(ChecksumAlgo::from_hex_len(64), Some(ChecksumAlgo::Sha256));
        assert_eq!(ChecksumAlgo::from_hex_len(128), Some(ChecksumAlgo::Sha512));
        assert_eq!(ChecksumAlgo::from_hex_len(96), Some(ChecksumAlgo::Sha384));
        assert_eq!(ChecksumAlgo::from_hex_len(56), Some(ChecksumAlgo::Sha224));
        assert_eq!(ChecksumAlgo::from_hex_len(40), None); // sha1 — unsupported
        assert_eq!(ChecksumAlgo::from_hex_len(32), None); // md5 — unsupported
    }

    #[test]
    fn digest_algo_rejects_non_hex() {
        assert!(digest_algo(&"z".repeat(64)).is_none());
        assert!(digest_algo("").is_none());
        assert_eq!(digest_algo(&"0".repeat(128)), Some(ChecksumAlgo::Sha512));
    }

    #[test]
    fn sha512_digest_matches_known_vector() {
        // SHA-512 of the empty input.
        let empty = ChecksumAlgo::Sha512.digest_hex(b"");
        assert_eq!(
            empty,
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
    }

    #[test]
    fn prefers_sidecar_over_generic_list() {
        let assets = vec![
            asset("tool.tar.gz"),
            asset("checksums.txt"),
            asset("tool.tar.gz.sha512"),
        ];
        let found = find_checksum_asset("tool.tar.gz", &assets).unwrap();
        assert_eq!(found.name, "tool.tar.gz.sha512");
    }

    #[test]
    fn falls_back_to_generic_list() {
        let assets = vec![asset("tool.tar.gz"), asset("SHA512SUMS")];
        let found = find_checksum_asset("tool.tar.gz", &assets).unwrap();
        assert_eq!(found.name, "SHA512SUMS");
    }

    #[test]
    fn recognizes_generic_checksum_filenames() {
        for name in [
            "checksums.txt",
            "tool_checksums.txt",
            "checksum",
            "0.1.1-checksums.txt",
            "SHA256SUMS",
            "sha512sums.txt",
            "sha256sum",
            "checksums.sha256",
            "SHA512SUMS",
        ] {
            assert!(is_generic_checksums_file(name), "{name} should match");
        }
        for name in ["tool.tar.gz", "tool.sig", "readme.txt"] {
            assert!(!is_generic_checksums_file(name), "{name} should not match");
        }
    }

    #[test]
    fn no_checksum_asset_returns_none() {
        let assets = vec![asset("tool.tar.gz"), asset("tool.tar.gz.sig")];
        assert!(find_checksum_asset("tool.tar.gz", &assets).is_none());
    }
}
