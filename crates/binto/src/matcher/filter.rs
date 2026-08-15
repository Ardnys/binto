use tracing::debug;

use crate::github::types::Asset;

const CHECKSUM_EXTENSIONS: &[&str] = &[
    ".sha256",
    ".sha512",
    ".sha1",
    ".md5",
    ".sig",
    ".asc",
    ".minisig",
    ".b64",
    ".sigstore.json",
];

// TODO: checksum names could be after the archive
// TODO: there's also sigstore.json
const CHECKSUM_NAMES: &[&str] = &[
    "checksums.txt",
    "sha256sums",
    "sha256sums.txt",
    "sha512sums",
    "sha512sums.txt",
    "sha256sum",
    "sha256sum.txt",
    "checksums.sha256",
    "checksums",
    "checksum.txt",
];

// TODO: also parse sbom.json files
// apparently they are good for security. It would be a good investigation to see
// which repositories ship it and stuff.

// TODO: there's winx, dragonfly
const OS_REJECT_TERMS: &[&str] = &[
    "windows", "darwin", "macos", "osx", "win32", "win64", "freebsd", "netbsd", "mac", "openbsd",
    "solaris", "android",
];

const OS_REJECT_EXTENSIONS: &[&str] = &[".exe", ".msi", ".dmg", ".pkg", ".apk"];

/// Remove GitHub-generated source archives (they have no arch/OS specificity).
pub fn filter_source_archives(assets: Vec<Asset>) -> Vec<Asset> {
    assets
        .into_iter()
        .filter(|a| {
            let n = a.name.to_lowercase();
            // GitHub auto-generates these; their browser_download_url is always the same
            // pattern. We detect them by content_type or by the generic naming convention.
            // content_type for source archives is "application/zip" or "application/x-tar"
            // but so are real assets. Most reliable signal: name matches {anything}-{tag}.tar.gz
            // with no other distinguishing terms. We use the absence of any OS/arch markers
            // as a secondary signal, but the primary filter is the GitHub source archive
            // content_type "application/x-zip-compressed" combined with a very generic name.
            //
            // Safest heuristic: GitHub source archives show up in the API as assets with
            // content_type "application/zip" and names like "Source code (zip)" or just
            // the pattern. In practice, the GitHub API does NOT include auto-generated
            // source archives in the `assets` array at all — they appear only in
            // tarball_url / zipball_url fields. So this filter is mostly a safety net
            // for manually uploaded source tarballs named like source archives.
            let keep = !is_source_archive(&n);
            if !keep {
                debug!(asset = %a.name, reason = "source_archive", "filtered out asset");
            }
            keep
        })
        .collect()
}

fn is_source_archive(name: &str) -> bool {
    // A manually uploaded source archive typically has a name like:
    // "{repo}-{version}.tar.gz" or "{repo}-{version}.zip"
    // with no arch/OS markers and no binary content.
    // We can't detect these with certainty without downloading, so we leave this
    // to the scoring pipeline (they'll score near zero without arch/OS terms).
    // The one case we can hard-filter: names that are exactly "source code" variants.
    name.contains("source code") || name.contains("source_code")
}

/// Remove checksum, signature, and verification sidecar files.
pub fn filter_checksums(assets: Vec<Asset>) -> Vec<Asset> {
    assets
        .into_iter()
        .filter(|a| {
            let lower = a.name.to_lowercase();
            let checksum_ext = CHECKSUM_EXTENSIONS.iter().find(|ext| lower.ends_with(*ext));
            let is_checksum_name = CHECKSUM_NAMES.iter().any(|name| lower == *name);
            let contains_checksum_name = lower.contains("checksum");
            if let Some(ext) = checksum_ext {
                debug!(asset = %a.name, reason = "checksum_extension", term = ext, "filtered out asset");
            } else if is_checksum_name {
                debug!(asset = %a.name, reason = "checksum_name", "filtered out asset");
            } else if contains_checksum_name {
                debug!(asset = %a.name, reason = "contains_checksum_name", "filtered out asset");
            }
            checksum_ext.is_none() && !is_checksum_name && !contains_checksum_name
        })
        .collect()
}

/// Remove assets targeting non-Linux operating systems.
pub fn filter_os(assets: Vec<Asset>) -> Vec<Asset> {
    assets
        .into_iter()
        .filter(|a| {
            let lower = a.name.to_lowercase();
            let reject_term = OS_REJECT_TERMS.iter().find(|t| lower.contains(*t));
            let reject_ext = OS_REJECT_EXTENSIONS
                .iter()
                .find(|ext| lower.ends_with(*ext));
            if let Some(term) = reject_term {
                debug!(asset = %a.name, reason = "os_reject_term", term, "filtered out asset");
            } else if let Some(ext) = reject_ext {
                debug!(asset = %a.name, reason = "os_reject_ext", term = ext, "filtered out asset");
            }
            reject_term.is_none() && reject_ext.is_none()
        })
        .collect()
}

/// Apply all hard filters in sequence.
pub fn apply_hard_filters(assets: Vec<Asset>) -> Vec<Asset> {
    let assets = filter_source_archives(assets);
    let assets = filter_checksums(assets);
    filter_os(assets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
            size: 1024,
            content_type: "application/octet-stream".to_string(),
        }
    }

    #[test]
    fn removes_checksum_files() {
        let assets = vec![
            asset("tool_linux_amd64.tar.gz"),
            asset("tool_linux_amd64.tar.gz.sha256"),
            asset("checksums.txt"),
            asset("tool_checksums.txt"),
            asset("SHA256SUMS"),
        ];
        let filtered = filter_checksums(assets);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "tool_linux_amd64.tar.gz");
    }

    #[test]
    fn removes_windows_darwin_assets() {
        let assets = vec![
            asset("tool_linux_amd64.tar.gz"),
            asset("tool_windows_amd64.zip"),
            asset("tool_darwin_arm64.tar.gz"),
            asset("tool_macos_arm64.tar.gz"),
            asset("tool_win64.exe"),
        ];
        let filtered = filter_os(assets);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "tool_linux_amd64.tar.gz");
    }
}
