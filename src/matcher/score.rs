use std::fmt::Display;

use tracing::debug;

use crate::config::Libc;
use crate::github::types::Asset;

// Scoring weights — tune here without touching logic
const SCORE_ARCH_EXACT: i32 = 1000;
const SCORE_ARCH_SYNONYM: i32 = 800;
const SCORE_LINUX_KEYWORD: i32 = 200;
const SCORE_PREFERRED_LIBC: i32 = 400;
const PENALTY_OTHER_LIBC: i32 = -100; // other libc gets a penalty, to satisfy the threshold
const SCORE_FORMAT_RAW: i32 = 50; // now raw and archives are very close and falls to interaction
const SCORE_FORMAT_TAR: i32 = 450;
const SCORE_FORMAT_ZIP: i32 = 50;
const SCORE_FORMAT_APPIMG: i32 = 10;
const SCORE_FORMAT_REJECT: i32 = -9999;
pub const CONFIDENCE_THRESHOLD: i32 = 400;
// TODO: there's also 32bit ones that we have to eliminate later probs

// TODO: there's mips and variants, ppc64
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

    // TODO: SBOM handling will be done later. Just reject them right now
    if name.ends_with("sbom.json") {
        total += SCORE_FORMAT_REJECT;
    }

    // Arch scoring
    let mut arch_term = "";
    let arch_score;
    let arch_match = if let Some(term) = user_syns.iter().find(|s| name.contains(*s)) {
        arch_term = term;
        // Check if it's the exact canonical form
        if name.contains(user_canonical) {
            arch_term = user_canonical;
            arch_score = SCORE_ARCH_EXACT;
            ArchMatch::Exact
        } else {
            arch_score = SCORE_ARCH_SYNONYM;
            ArchMatch::Synonym
        }
    } else if let Some(term) = foreign_terms.iter().find(|t| name.contains(*t)) {
        // Contains a term from a different arch — hard penalize
        arch_term = term;
        arch_score = SCORE_FORMAT_REJECT;
        ArchMatch::None
    } else {
        arch_score = 0;
        ArchMatch::None
    };
    total += arch_score;

    // Linux keyword bonus
    let linux_keyword = name.contains("linux");
    if linux_keyword {
        total += SCORE_LINUX_KEYWORD;
    }

    // libc preference — the preferred flavor earns the larger bonus, the other stays eligible
    // (smaller bonus) so a tool shipping only the non-preferred libc still installs.
    let is_gnu = name.contains("gnu") || name.contains("glibc");
    let is_musl = name.contains("musl") || name.contains("static");
    let libc_detected = if is_gnu {
        "gnu"
    } else if is_musl {
        "musl"
    } else {
        "none"
    };
    let libc_score = if is_gnu {
        if prefer_libc == Libc::Gnu {
            SCORE_PREFERRED_LIBC
        } else {
            PENALTY_OTHER_LIBC
        }
    } else if is_musl {
        if prefer_libc == Libc::Musl {
            SCORE_PREFERRED_LIBC
        } else {
            PENALTY_OTHER_LIBC
        }
    } else {
        0
    };
    total += libc_score;

    // Format scoring — strip from right to handle compound extensions
    let (format, format_score) = if name.ends_with(".deb") || name.ends_with(".rpm") {
        ("reject", SCORE_FORMAT_REJECT)
    } else if name.ends_with(".tar.gz")
        || name.ends_with(".tar.xz")
        || name.ends_with(".tar.zst")
        || name.ends_with(".tar.bz2")
        || name.ends_with(".tgz")
    // TODO: there's also .bz2, .gz
    {
        ("tar", SCORE_FORMAT_TAR)
    } else if name.ends_with(".zip") {
        ("zip", SCORE_FORMAT_ZIP)
    } else if name.ends_with(".appimage") {
        ("appimage", SCORE_FORMAT_APPIMG)
    } else {
        // No known archive extension → treat as raw binary
        ("raw", SCORE_FORMAT_RAW)
    };
    total += format_score;

    debug!(
        asset = %asset.name,
        arch_match = %arch_match,
        arch_term,
        arch_score,
        linux_keyword,
        libc_detected,
        libc_score,
        format,
        format_score,
        total,
        "scored asset"
    );

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
    let mut scored: Vec<ScoredAsset> = assets
        .into_iter()
        .map(|a| {
            let score = score_asset(&a, user_arch, prefer_libc);
            ScoredAsset { asset: a, score }
        })
        .filter(|s| {
            let keep = s.score.total > 0;
            if !keep {
                debug!(
                    asset = %s.asset.name,
                    total = s.score.total,
                    reason = "non_positive_score",
                    "asset excluded from ranking"
                );
            }
            keep
        })
        .collect();

    scored.sort_by_key(|b| std::cmp::Reverse(b.score.total));
    for (rank, s) in scored.iter().enumerate() {
        debug!(
            asset = %s.asset.name,
            rank,
            total = s.score.total,
            arch_match = %s.score.arch_match,
            "ranked asset"
        );
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
