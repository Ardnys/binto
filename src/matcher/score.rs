use std::fmt::Display;

use tracing::{Span, debug};

use crate::config::Libc;
use crate::github::types::Asset;

// Scoring weights — tune here without touching logic
const SCORE_ARCH_EXACT: i32 = 1000;
const SCORE_ARCH_SYNONYM: i32 = 800;
const SCORE_LINUX_KEYWORD: i32 = 200;
const SCORE_PREFERRED_LIBC: i32 = 400;
const PENALTY_OTHER_LIBC: i32 = -100; // other libc gets a penalty, to satisfy the threshold
const SCORE_FORMAT_RAW: i32 = 400;
const SCORE_FORMAT_TAR: i32 = 300;
const SCORE_FORMAT_ZIP: i32 = 200;
const SCORE_FORMAT_APPIMG: i32 = 100;
const SCORE_FORMAT_REJECT: i32 = -9999;
pub const CONFIDENCE_THRESHOLD: i32 = 400;

const ARCH_SYNONYMS: &[(&str, &[&str])] = &[
    ("x86_64", &["x86_64", "amd64", "x64", "amd_64"]),
    ("aarch64", &["aarch64", "arm64"]),
    ("armv7", &["armv7", "armv7l", "armhf", "arm"]),
    ("i686", &["i686", "i386", "x86", "386"]),
    // Non-x86 arches we never run on here — listed so they land in the foreign-reject set and
    // can't be mis-picked on an x86_64/aarch64 host.
    ("riscv64", &["riscv64"]),
    ("ppc64le", &["ppc64le", "powerpc64le"]),
    ("s390x", &["s390x"]),
    ("loongarch64", &["loongarch64", "loong64"]),
];

#[derive(Debug, Clone, PartialEq)]
pub enum ArchMatch {
    Exact,
    Synonym,
    None,
}

impl Display for ArchMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchMatch::Exact => write!(f, "EXACT"),
            ArchMatch::Synonym => write!(f, "SYNONYM"),
            ArchMatch::None => write!(f, "NONE"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AssetScore {
    pub arch_match: ArchMatch,
    pub total: i32,
}

pub fn detect_arch() -> String {
    std::process::Command::new("uname")
        .arg("-m")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_else(|| std::env::consts::ARCH.to_lowercase())
}

fn arch_synonyms_for(canonical: &str) -> &'static [&'static str] {
    ARCH_SYNONYMS
        .iter()
        .find(|(c, _)| *c == canonical)
        .map(|(_, syns)| *syns)
        .unwrap_or(&[])
}

/// All synonym terms for arches OTHER than the user's.
fn foreign_arch_terms(user_canonical: &str) -> Vec<&'static str> {
    ARCH_SYNONYMS
        .iter()
        .filter(|(c, _)| *c != user_canonical)
        .flat_map(|(_, syns)| syns.iter().copied())
        .collect()
}

fn canonical_arch(raw: &str) -> &'static str {
    let raw = raw.trim().to_lowercase();
    for (canonical, syns) in ARCH_SYNONYMS {
        if syns.contains(&raw.as_str()) {
            return canonical;
        }
    }
    // fallback to x86_64
    "x86_64"
}

pub fn score_asset(asset: &Asset, user_arch_raw: &str, prefer_libc: Libc) -> AssetScore {
    let name = asset.name.to_lowercase();
    let user_canonical = canonical_arch(user_arch_raw);
    let user_syns = arch_synonyms_for(user_canonical);
    let foreign_terms = foreign_arch_terms(user_canonical);

    let mut total = 0i32;

    // Arch scoring
    let arch_match = if user_syns.iter().any(|s| name.contains(s)) {
        // Check if it's the exact canonical form
        if name.contains(user_canonical) {
            total += SCORE_ARCH_EXACT;
            ArchMatch::Exact
        } else {
            total += SCORE_ARCH_SYNONYM;
            ArchMatch::Synonym
        }
    } else if foreign_terms.iter().any(|t| name.contains(t)) {
        // Contains a term from a different arch — hard penalize
        total += SCORE_FORMAT_REJECT;
        ArchMatch::None
    } else {
        ArchMatch::None
    };

    // Linux keyword bonus
    if name.contains("linux") {
        total += SCORE_LINUX_KEYWORD;
    }

    // libc preference — the preferred flavor earns the larger bonus, the other stays eligible
    // (smaller bonus) so a tool shipping only the non-preferred libc still installs.
    let is_gnu = name.contains("gnu") || name.contains("glibc");
    let is_musl = name.contains("musl") || name.contains("static");
    if is_gnu {
        // score preferred libc higher
        total += if prefer_libc == Libc::Gnu {
            SCORE_PREFERRED_LIBC
        } else {
            PENALTY_OTHER_LIBC
        };
    } else if is_musl {
        // score preferred libc higher
        total += if prefer_libc == Libc::Musl {
            SCORE_PREFERRED_LIBC
        } else {
            PENALTY_OTHER_LIBC
        };
    }

    // Format scoring — strip from right to handle compound extensions
    if name.ends_with(".deb") || name.ends_with(".rpm") {
        total += SCORE_FORMAT_REJECT;
    } else if name.ends_with(".tar.gz")
        || name.ends_with(".tar.xz")
        || name.ends_with(".tar.bz2")
        || name.ends_with(".tgz")
    {
        total += SCORE_FORMAT_TAR;
    } else if name.ends_with(".zip") {
        total += SCORE_FORMAT_ZIP;
    } else if name.to_lowercase().ends_with(".appimage") {
        total += SCORE_FORMAT_APPIMG;
    } else {
        // No known archive extension → treat as raw binary
        total += SCORE_FORMAT_RAW;
    }

    AssetScore { arch_match, total }
}

#[derive(Debug)]
pub struct ScoredAsset {
    pub asset: Asset,
    pub score: AssetScore,
}

/// Score and sort a list of pre-filtered assets. Returns sorted descending by score.
/// Assets with SCORE_FORMAT_REJECT or foreign-arch penalty are excluded.
pub fn score_and_rank(assets: Vec<Asset>, user_arch: &str, prefer_libc: Libc) -> Vec<ScoredAsset> {
    let _span = Span::current();
    let mut scored: Vec<ScoredAsset> = assets
        .into_iter()
        .map(|a| {
            let score = score_asset(&a, user_arch, prefer_libc);
            ScoredAsset { asset: a, score }
        })
        .filter(|s| s.score.total > 0)
        .collect();

    scored.sort_by_key(|b| std::cmp::Reverse(b.score.total));
    for s in &scored {
        // print each asset and their score, good for debugging strange asset scores
        debug!("Asset {:?} - Score: {:?}", s.asset.name, s.score);
    }
    scored
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
    fn prefers_exact_arch_over_synonym() {
        let s_exact = score_asset(&asset("tool_x86_64_linux.tar.gz"), "x86_64", Libc::Gnu);
        let s_synonym = score_asset(&asset("tool_amd64_linux.tar.gz"), "x86_64", Libc::Gnu);
        assert!(s_exact.total > s_synonym.total);
    }

    #[test]
    fn prefers_gnu_over_musl() {
        let gnu = score_asset(&asset("tool_x86_64_linux_gnu.tar.gz"), "x86_64", Libc::Gnu);
        let musl = score_asset(&asset("tool_x86_64_linux_musl.tar.gz"), "x86_64", Libc::Gnu);
        assert!(gnu.total > musl.total);
    }

    #[test]
    fn rejects_foreign_arch() {
        let arm = score_asset(&asset("tool_aarch64_linux.tar.gz"), "x86_64", Libc::Gnu);
        assert!(arm.total <= 0);
    }

    #[test]
    fn rejects_deb_rpm() {
        let deb = score_asset(&asset("tool_amd64.deb"), "x86_64", Libc::Gnu);
        let rpm = score_asset(&asset("tool_x86_64.rpm"), "x86_64", Libc::Gnu);
        assert!(deb.total < 0);
        assert!(rpm.total < 0);
    }

    #[test]
    fn raw_binary_scores_higher_than_appimage() {
        let raw = score_asset(&asset("tool_x86_64_linux"), "x86_64", Libc::Gnu);
        let appimg = score_asset(&asset("Tool-x86_64.AppImage"), "x86_64", Libc::Gnu);
        assert!(raw.total > appimg.total);
    }

    // Real-world fixture: ripgrep release assets
    #[test]
    fn ripgrep_selects_gnu_tarball_on_x86_64() {
        let candidates = vec![
            asset("ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz"),
            asset("ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz"),
            asset("ripgrep-14.1.0-aarch64-unknown-linux-gnu.tar.gz"),
            asset("ripgrep-14.1.0-x86_64-pc-windows-msvc.zip"),
        ];
        let ranked = score_and_rank(candidates, "x86_64", Libc::Gnu);
        assert!(!ranked.is_empty());
        assert_eq!(
            ranked[0].asset.name,
            "ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    // Real-world fixture: gh CLI release assets
    #[test]
    fn gh_cli_selects_linux_amd64_tarball() {
        let candidates = vec![
            asset("gh_2.45.0_linux_amd64.tar.gz"),
            asset("gh_2.45.0_linux_arm64.tar.gz"),
            asset("gh_2.45.0_linux_386.tar.gz"),
            asset("gh_2.45.0_windows_amd64.zip"),
            asset("gh_2.45.0_macOS_amd64.zip"),
        ];
        let ranked = score_and_rank(candidates, "x86_64", Libc::Gnu);
        assert!(!ranked.is_empty());
        assert_eq!(ranked[0].asset.name, "gh_2.45.0_linux_amd64.tar.gz");
    }

    // Real-world fixture: bat release assets
    #[test]
    fn bat_selects_x86_64_gnu_tarball() {
        let candidates = vec![
            asset("bat-v0.24.0-x86_64-unknown-linux-gnu.tar.gz"),
            asset("bat-v0.24.0-x86_64-unknown-linux-musl.tar.gz"),
            asset("bat-v0.24.0-aarch64-unknown-linux-gnu.tar.gz"),
            asset("bat-v0.24.0-arm-unknown-linux-gnueabihf.tar.gz"),
            asset("bat-v0.24.0-x86_64-apple-darwin.tar.gz"),
        ];
        let ranked = score_and_rank(candidates, "x86_64", Libc::Gnu);
        assert!(!ranked.is_empty());
        assert_eq!(
            ranked[0].asset.name,
            "bat-v0.24.0-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    // Real-world fixture: delta (git-delta) release assets
    #[test]
    fn delta_selects_x86_64_musl_when_only_option() {
        let candidates = vec![
            asset("delta-0.17.0-x86_64-unknown-linux-musl.tar.gz"),
            asset("delta-0.17.0-aarch64-unknown-linux-gnu.tar.gz"),
            asset("delta-0.17.0-x86_64-apple-darwin.tar.gz"),
            asset("delta-0.17.0-x86_64-pc-windows-msvc.zip"),
        ];
        let ranked = score_and_rank(candidates, "x86_64", Libc::Gnu);
        assert!(!ranked.is_empty());
        assert_eq!(
            ranked[0].asset.name,
            "delta-0.17.0-x86_64-unknown-linux-musl.tar.gz"
        );
    }

    // aarch64 host should select arm64 assets
    #[test]
    fn aarch64_host_selects_arm64_asset() {
        let candidates = vec![
            asset("tool-linux-amd64.tar.gz"),
            asset("tool-linux-arm64.tar.gz"),
        ];
        let ranked = score_and_rank(candidates, "aarch64", Libc::Gnu);
        assert!(!ranked.is_empty());
        assert_eq!(ranked[0].asset.name, "tool-linux-arm64.tar.gz");
    }

    // Confidence gap: gnu vs musl is a gap of 500, above threshold
    // needed to pass the interaction
    #[test]
    fn gnu_vs_musl_gap_is_above_confidence_threshold() {
        let gnu = score_asset(&asset("tool_x86_64_linux_gnu.tar.gz"), "x86_64", Libc::Gnu);
        let musl = score_asset(&asset("tool_x86_64_linux_musl.tar.gz"), "x86_64", Libc::Gnu);
        assert!((gnu.total - musl.total) > CONFIDENCE_THRESHOLD);
    }

    // Confidence gap: exact arch vs synonym is 200, below threshold
    #[test]
    fn exact_vs_synonym_gap_is_below_confidence_threshold() {
        let exact = score_asset(&asset("tool_x86_64_linux.tar.gz"), "x86_64", Libc::Gnu);
        let synonym = score_asset(&asset("tool_amd64_linux.tar.gz"), "x86_64", Libc::Gnu);
        assert!((exact.total - synonym.total) < CONFIDENCE_THRESHOLD);
    }

    // prefer_libc = Musl flips ripgrep's top pick to the musl tarball
    #[test]
    fn musl_preference_selects_musl_tarball() {
        let candidates = vec![
            asset("ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz"),
            asset("ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz"),
        ];
        let ranked = score_and_rank(candidates, "x86_64", Libc::Musl);
        assert_eq!(
            ranked[0].asset.name,
            "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz"
        );
    }

    // Non-x86 arches must be rejected on an x86_64 host so they're never mis-picked
    #[test]
    fn rejects_non_x86_arches() {
        for name in [
            "tool-riscv64-unknown-linux-gnu.tar.gz",
            "tool-s390x-unknown-linux-gnu.tar.gz",
            "tool-ppc64le-unknown-linux-gnu.tar.gz",
            "tool-loongarch64-unknown-linux-gnu.tar.gz",
            "tool-armv7l-unknown-linux-gnueabihf.tar.gz",
        ] {
            let s = score_asset(&asset(name), "x86_64", Libc::Gnu);
            assert!(s.total <= 0, "{name} should be rejected on x86_64");
        }
    }
}
