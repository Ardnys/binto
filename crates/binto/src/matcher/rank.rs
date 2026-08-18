//! Preference tiers: how binto chooses between assets it *could* install.
//!
//! Everything reaching this module has already cleared [`super::filter`], so every
//! candidate is installable. What remains is preference, and preference is expressed as
//! an ordering rather than a weight. A tier only ever loses to a *better* tier on the
//! same dimension — there is no arithmetic in which a strong format outvotes a wrong
//! libc, and no threshold to tune.
//!
//! Two candidates that land on identical tiers are genuinely indistinguishable to binto.
//! That is reported as ambiguity, not resolved by a tiebreaker nobody can audit.

use tracing::debug;

use crate::config::Libc;
use crate::matcher::facts::{ArchFact, Format, LibcFact, OsFact};
use crate::matcher::filter::Candidate;

/// The ordered preferences binto ranks by.
///
/// Plain data with no behaviour of its own, so a `[preferences]` config table can
/// construct one later without touching the ranking logic.
#[derive(Debug, Clone)]
pub struct PreferenceProfile {
    /// Which libc to favour when a release ships more than one.
    pub libc: Libc,
    /// Packaging formats, best first. A format missing from this list ranks last.
    pub format_order: Vec<Format>,
}

impl PreferenceProfile {
    /// The built-in defaults.
    ///
    /// Formats: tarballs first (they carry the executable bit and usually the man pages
    /// and completions too), then zip, then a bare binary, then AppImage.
    pub fn new(libc: Libc) -> Self {
        Self {
            libc,
            format_order: vec![Format::Tar, Format::Zip, Format::Raw, Format::AppImage],
        }
    }
}

/// Where a candidate lands on each dimension, `0` being best.
///
/// **Field order is priority order.** The derived `Ord` compares lexicographically, so
/// architecture specificity outranks OS, which outranks libc, which outranks packaging —
/// and the smallest `Tiers` is the winner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tiers {
    pub arch: u8,
    pub os: u8,
    pub libc: u8,
    pub format: u8,
}

/// Something about the winning asset the user should know before it is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionNote {
    /// A preference existed and the release could not satisfy it — you asked for gnu and
    /// only musl was published. The install is still fine; it is just not what you asked
    /// for, and saying so is the whole point.
    Fallback {
        dimension: &'static str,
        wanted: &'static str,
        got: &'static str,
    },
    /// The winner names nothing at all on this dimension, so binto is trusting the
    /// publisher rather than verifying a marker.
    Unspecified { dimension: &'static str },
}

/// A candidate placed on the preference tiers.
#[derive(Debug, Clone)]
pub struct RankedAsset {
    pub candidate: Candidate,
    pub tiers: Tiers,
}

impl RankedAsset {
    pub fn name(&self) -> &str {
        &self.candidate.asset.name
    }

    /// Tier labels for the trace and the verdict, in priority order.
    pub fn labels(&self) -> [(&'static str, &'static str); 4] {
        [
            ("arch", self.candidate.arch.label()),
            ("os", self.candidate.os.label()),
            ("libc", self.candidate.libc.label()),
            ("format", self.candidate.format.label()),
        ]
    }
}

/// An asset that states its architecture is preferred over one that leaves it to chance.
fn arch_tier(arch: ArchFact) -> u8 {
    match arch {
        ArchFact::Named(_) => 0,
        ArchFact::Unspecified => 1,
    }
}

fn os_tier(os: OsFact) -> u8 {
    match os {
        OsFact::Linux => 0,
        OsFact::Unspecified => 1,
        // Filtered out long before here; ranked last for completeness.
        OsFact::Foreign(_) => 2,
    }
}

/// Preferred libc first, then a build that names none, then the other flavour. An
/// unmarked build is ranked above the wrong flavour because it is more often a portable
/// static build than a mislabelled one.
fn libc_tier(libc: LibcFact, preferred: Libc) -> u8 {
    let wanted = match preferred {
        Libc::Gnu => LibcFact::Gnu,
        Libc::Musl => LibcFact::Musl,
    };
    match libc {
        _ if libc == wanted => 0,
        LibcFact::Unspecified => 1,
        _ => 2,
    }
}

fn format_tier(format: Format, order: &[Format]) -> u8 {
    order
        .iter()
        .position(|f| *f == format)
        .unwrap_or(order.len()) as u8
}

pub fn tiers_for(candidate: &Candidate, profile: &PreferenceProfile) -> Tiers {
    Tiers {
        arch: arch_tier(candidate.arch),
        os: os_tier(candidate.os),
        libc: libc_tier(candidate.libc, profile.libc),
        format: format_tier(candidate.format, &profile.format_order),
    }
}

/// Rank every candidate, best first.
pub fn rank(candidates: Vec<Candidate>, profile: &PreferenceProfile) -> Vec<RankedAsset> {
    let mut ranked: Vec<RankedAsset> = candidates
        .into_iter()
        .map(|candidate| {
            let tiers = tiers_for(&candidate, profile);
            RankedAsset { candidate, tiers }
        })
        .collect();

    ranked.sort_by_key(|r| r.tiers);

    for r in &ranked {
        let [(_, arch), (_, os), (_, libc), (_, format)] = r.labels();
        debug!(
            asset = %r.name(),
            arch, os, libc, format,
            arch_tier = r.tiers.arch,
            os_tier = r.tiers.os,
            libc_tier = r.tiers.libc,
            format_tier = r.tiers.format,
            "asset ranked"
        );
    }

    ranked
}

/// How many leading candidates share the best tiers. Zero only for an empty list.
///
/// This replaces the old confidence-gap arithmetic: a tie means binto has no principled
/// reason to prefer either asset, which is a statement about the release rather than
/// about a threshold.
pub fn tie_group_len(ranked: &[RankedAsset]) -> usize {
    match ranked.first() {
        None => 0,
        Some(best) => ranked.iter().take_while(|r| r.tiers == best.tiers).count(),
    }
}

/// What the user should know about `winner`, given what the release actually offered.
pub fn notes_for(winner: &RankedAsset, profile: &PreferenceProfile) -> Vec<SelectionNote> {
    let mut notes = Vec::new();

    if winner.tiers.libc > 0 {
        let wanted = match profile.libc {
            Libc::Gnu => "gnu",
            Libc::Musl => "musl",
        };
        match winner.candidate.libc {
            LibcFact::Unspecified => notes.push(SelectionNote::Unspecified { dimension: "libc" }),
            got => notes.push(SelectionNote::Fallback {
                dimension: "libc",
                wanted,
                got: got.label(),
            }),
        }
    }
    if winner.candidate.arch == ArchFact::Unspecified {
        notes.push(SelectionNote::Unspecified { dimension: "arch" });
    }
    if winner.candidate.os == OsFact::Unspecified {
        notes.push(SelectionNote::Unspecified { dimension: "os" });
    }

    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::types::Asset;
    use crate::matcher::filter::apply_hard_filters;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
            size: 1024,
            content_type: "application/octet-stream".to_string(),
        }
    }

    /// Filter then rank, as `match_asset` does — tests state release contents, not
    /// pre-filtered candidates.
    fn ranked(names: &[&str], host_arch: &str, libc: Libc) -> Vec<RankedAsset> {
        let assets = names.iter().map(|n| asset(n)).collect();
        let (candidates, _) = apply_hard_filters(assets, host_arch);
        rank(candidates, &PreferenceProfile::new(libc))
    }

    fn winner(names: &[&str], host_arch: &str, libc: Libc) -> String {
        ranked(names, host_arch, libc)[0].name().to_string()
    }

    #[test]
    fn ripgrep_selects_the_gnu_tarball_on_x86_64() {
        assert_eq!(
            winner(
                &[
                    "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz",
                    "ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz",
                    "ripgrep-14.1.0-aarch64-unknown-linux-gnu.tar.gz",
                    "ripgrep-14.1.0-x86_64-pc-windows-msvc.zip",
                ],
                "x86_64",
                Libc::Gnu
            ),
            "ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn gh_cli_selects_the_linux_amd64_tarball() {
        assert_eq!(
            winner(
                &[
                    "gh_2.45.0_linux_amd64.tar.gz",
                    "gh_2.45.0_linux_arm64.tar.gz",
                    "gh_2.45.0_linux_386.tar.gz",
                    "gh_2.45.0_windows_amd64.zip",
                    "gh_2.45.0_macOS_amd64.zip",
                ],
                "x86_64",
                Libc::Gnu
            ),
            "gh_2.45.0_linux_amd64.tar.gz"
        );
    }

    #[test]
    fn bat_selects_the_x86_64_gnu_tarball() {
        assert_eq!(
            winner(
                &[
                    "bat-v0.24.0-x86_64-unknown-linux-gnu.tar.gz",
                    "bat-v0.24.0-x86_64-unknown-linux-musl.tar.gz",
                    "bat-v0.24.0-aarch64-unknown-linux-gnu.tar.gz",
                    "bat-v0.24.0-arm-unknown-linux-gnueabihf.tar.gz",
                    "bat-v0.24.0-x86_64-apple-darwin.tar.gz",
                ],
                "x86_64",
                Libc::Gnu
            ),
            "bat-v0.24.0-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn aarch64_host_selects_the_arm64_asset() {
        assert_eq!(
            winner(
                &["tool-linux-amd64.tar.gz", "tool-linux-arm64.tar.gz"],
                "aarch64",
                Libc::Gnu
            ),
            "tool-linux-arm64.tar.gz"
        );
    }

    #[test]
    fn musl_preference_flips_the_pick() {
        let names = [
            "ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz",
            "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz",
        ];
        assert_eq!(
            winner(&names, "x86_64", Libc::Musl),
            "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            winner(&names, "x86_64", Libc::Gnu),
            "ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn libc_outranks_packaging() {
        // A gnu raw binary beats a musl tarball: libc sits above format in `Tiers`.
        assert_eq!(
            winner(
                &["tool-x86_64-linux-gnu", "tool-x86_64-linux-musl.tar.gz"],
                "x86_64",
                Libc::Gnu
            ),
            "tool-x86_64-linux-gnu"
        );
    }

    #[test]
    fn format_order_is_tar_zip_raw_appimage() {
        let ranked = ranked(
            &[
                "tool-x86_64-linux.AppImage",
                "tool-x86_64-linux",
                "tool-x86_64-linux.zip",
                "tool-x86_64-linux.tar.gz",
            ],
            "x86_64",
            Libc::Gnu,
        );
        let order: Vec<&str> = ranked.iter().map(|r| r.name()).collect();
        assert_eq!(
            order,
            vec![
                "tool-x86_64-linux.tar.gz",
                "tool-x86_64-linux.zip",
                "tool-x86_64-linux",
                "tool-x86_64-linux.AppImage",
            ]
        );
    }

    #[test]
    fn an_asset_naming_its_arch_beats_one_that_does_not() {
        assert_eq!(
            winner(
                &["tool-linux.tar.gz", "tool-x86_64-linux.tar.gz"],
                "x86_64",
                Libc::Gnu
            ),
            "tool-x86_64-linux.tar.gz"
        );
    }

    #[test]
    fn indistinguishable_assets_tie_instead_of_being_broken_arbitrarily() {
        let ranked = ranked(
            &[
                "tool-x86_64-linux-gnu.tar.gz",
                "tool-x86_64-linux-gnu-v3.tar.gz",
            ],
            "x86_64",
            Libc::Gnu,
        );
        assert_eq!(tie_group_len(&ranked), 2);
    }

    #[test]
    fn a_clear_winner_is_not_a_tie() {
        let ranked = ranked(
            &[
                "tool-x86_64-linux-gnu.tar.gz",
                "tool-x86_64-linux-musl.tar.gz",
            ],
            "x86_64",
            Libc::Gnu,
        );
        assert_eq!(tie_group_len(&ranked), 1);
    }

    /// The complaint that started the refactor: a musl-only release used to auto-select
    /// with nothing to show the preference had gone unmet.
    #[test]
    fn a_musl_only_release_records_the_libc_fallback() {
        let profile = PreferenceProfile::new(Libc::Gnu);
        let ranked = ranked(
            &[
                "delta-0.17.0-x86_64-unknown-linux-musl.tar.gz",
                "delta-0.17.0-aarch64-unknown-linux-gnu.tar.gz",
                "delta-0.17.0-x86_64-apple-darwin.tar.gz",
            ],
            "x86_64",
            Libc::Gnu,
        );
        assert_eq!(
            ranked[0].name(),
            "delta-0.17.0-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            notes_for(&ranked[0], &profile),
            vec![SelectionNote::Fallback {
                dimension: "libc",
                wanted: "gnu",
                got: "musl",
            }]
        );
    }

    #[test]
    fn a_satisfied_preference_produces_no_notes() {
        let profile = PreferenceProfile::new(Libc::Gnu);
        let ranked = ranked(&["tool-x86_64-linux-gnu.tar.gz"], "x86_64", Libc::Gnu);
        assert_eq!(notes_for(&ranked[0], &profile), vec![]);
    }

    #[test]
    fn an_unmarked_asset_says_what_it_left_unstated() {
        let profile = PreferenceProfile::new(Libc::Gnu);
        let ranked = ranked(&["tool"], "x86_64", Libc::Gnu);
        assert_eq!(
            notes_for(&ranked[0], &profile),
            vec![
                SelectionNote::Unspecified { dimension: "libc" },
                SelectionNote::Unspecified { dimension: "arch" },
                SelectionNote::Unspecified { dimension: "os" },
            ]
        );
    }
}
