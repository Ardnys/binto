pub mod facts;
pub mod filter;
pub mod pattern;
pub mod rank;

use std::collections::HashSet;

use anyhow::Result;
use tracing::debug;

use crate::config::Libc;
use crate::error::BintoError;
use crate::github::types::Asset;
use filter::apply_hard_filters;
use rank::{PreferenceProfile, RankedAsset, SelectionNote, notes_for, rank, tie_group_len};

/// The asset binto chose, with everything the caller needs to explain the choice.
#[derive(Debug)]
pub struct Selection {
    pub ranked: RankedAsset,
    /// Preferences the release could not satisfy, and dimensions it left unstated.
    pub notes: Vec<SelectionNote>,
    /// The chosen asset's stem, but only when the release ships more than one distinct
    /// binary — `Some("tool-server")` for a repo publishing `tool-cli` beside it.
    ///
    /// `None` for the overwhelming majority of releases, where every candidate reduces to
    /// the same stem. The repo-derived name is already right there, and naming by stem
    /// would rename tools for no gain — `GitoxideLabs/gitoxide` would become
    /// `gitoxide-max-pure`.
    pub variant: Option<String>,
}

impl Selection {
    pub fn asset(&self) -> &Asset {
        &self.ranked.candidate.asset
    }
}

#[derive(Debug)]
pub enum MatchOutput {
    AutoSelected(Selection),
    /// Several assets landed on identical preference tiers — binto has no principled
    /// reason to prefer any of them.
    NeedsInteraction {
        /// Every candidate, best first; the tied ones lead.
        ranked: Vec<RankedAsset>,
        /// Notes for the leader. The tie group shares its tiers, so these describe what
        /// the whole group has in common — often the reason it tied.
        notes: Vec<SelectionNote>,
    },
}

/// Main entry point for asset matching.
///
/// If `stored_pattern` is provided (from a previous install), try the pattern fast-path
/// first. Falls back to the full filter-and-rank pipeline if the pattern matches zero or
/// multiple assets.
#[tracing::instrument(
    skip_all,
    fields(repo = repo, tag = tag, arch = user_arch, libc = ?prefer_libc)
)]
pub fn match_asset(
    all_assets: Vec<Asset>,
    user_arch: &str,
    stored_pattern: Option<&str>,
    repo: &str,
    tag: &str,
    prefer_libc: Libc,
) -> Result<MatchOutput> {
    let profile = PreferenceProfile::new(prefer_libc);

    // Pattern fast-path: if we have a stored pattern and it matches exactly one asset.
    if let Some(pat) = stored_pattern
        && let Some(selection) = pattern_fast_path(pat, &all_assets, user_arch, tag, &profile)
    {
        return Ok(MatchOutput::AutoSelected(selection));
    }

    let total_assets = all_assets.len();
    let (candidates, rejected) = apply_hard_filters(all_assets, user_arch, Some(tag));
    debug!(
        before = total_assets,
        after = candidates.len(),
        rejected = rejected.len(),
        "applied hard filters"
    );

    if candidates.is_empty() {
        debug!(outcome = "no_match", "selection");
        return Err(BintoError::NoCompatibleAssets {
            repo: repo.to_string(),
            tag: tag.to_string(),
        }
        .into());
    }

    let ranked = rank(candidates, &profile);
    let tied = tie_group_len(&ranked);
    let ships_variants = distinct_stems(&ranked) > 1;

    if tied == 1 {
        let winner = ranked
            .into_iter()
            .next()
            .expect("tie group of 1 is non-empty");
        let notes = notes_for(&winner, &profile);
        trace_selection("auto_selected", &winner, tied, &notes);
        let variant = variant_of(&winner, ships_variants);
        return Ok(MatchOutput::AutoSelected(Selection {
            ranked: winner,
            notes,
            variant,
        }));
    }

    let notes = notes_for(&ranked[0], &profile);
    trace_selection("needs_interaction", &ranked[0], tied, &notes);
    Ok(MatchOutput::NeedsInteraction { ranked, notes })
}

/// How many distinct binaries a release ships, read off the candidate stems.
///
/// Assets whose stem came out empty are not counted: a nameless asset says nothing about
/// how many binaries there are, and counting it would make a one-binary release look like
/// two and rename the tool to the empty string.
pub fn distinct_stems(ranked: &[RankedAsset]) -> usize {
    ranked
        .iter()
        .map(|r| r.stem())
        .filter(|stem| !stem.is_empty())
        .collect::<HashSet<_>>()
        .len()
}

/// The stem to name an asset by, or `None` to leave naming to the repo.
fn variant_of(chosen: &RankedAsset, ships_variants: bool) -> Option<String> {
    (ships_variants && !chosen.stem().is_empty()).then(|| chosen.stem().to_string())
}

/// Re-select the asset a previous install chose. Returns `None` when the pattern does not
/// pin exactly one asset, or when that asset would not survive the hard filters — a
/// stored pattern is a shortcut, never a licence to install something unusable.
fn pattern_fast_path(
    pat: &str,
    all_assets: &[Asset],
    user_arch: &str,
    tag: &str,
    profile: &PreferenceProfile,
) -> Option<Selection> {
    let names: Vec<&str> = all_assets.iter().map(|a| a.name.as_str()).collect();
    let matched = pattern::match_pattern(pat, &names);

    if matched.len() != 1 {
        debug!(
            pattern = pat,
            match_count = matched.len(),
            "pattern fast-path inconclusive, falling back to ranking"
        );
        return None;
    }

    let asset = all_assets.iter().find(|a| a.name == matched[0])?.clone();
    let (mut candidates, _) = apply_hard_filters(vec![asset], user_arch, Some(tag));
    let candidate = candidates.pop().or_else(|| {
        debug!(
            pattern = pat,
            asset = matched[0],
            "pattern fast-path hit an asset that fails the hard filters, falling back to ranking"
        );
        None
    })?;

    let tiers = rank::tiers_for(&candidate, profile);
    let winner = RankedAsset { candidate, tiers };
    let notes = notes_for(&winner, profile);
    debug!(pattern = pat, asset = %winner.name(), "pattern fast-path selected asset");
    trace_selection("auto_selected", &winner, 1, &notes);

    Some(Selection {
        ranked: winner,
        notes,
        // The fast path deliberately looks at one asset, so it cannot see whether the
        // release ships others. It only runs on update, where the name to install under is
        // already recorded on the tool and never re-derived.
        variant: None,
    })
}

fn trace_selection(outcome: &str, winner: &RankedAsset, tied: usize, notes: &[SelectionNote]) {
    let [(_, arch), (_, os), (_, libc), (_, format)] = winner.labels();
    debug!(
        outcome,
        asset = %winner.name(),
        tied,
        arch, os, libc, format,
        notes = ?notes,
        "selection"
    );
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

    fn assets(names: &[&str]) -> Vec<Asset> {
        names.iter().map(|n| asset(n)).collect()
    }

    /// The bug this whole change exists for: `tool-cli` and `tool-server` both derived
    /// their install name from the repo, so the second overwrote the first in state.
    /// Naming them after their own assets is what keeps them apart.
    #[test]
    fn a_release_shipping_several_binaries_names_each_after_its_own_asset() {
        let out = match_asset(
            assets(&[
                "tool-cli-1.2.3-x86_64-unknown-linux-gnu.tar.gz",
                "tool-server-1.2.3-x86_64-unknown-linux-gnu.tar.gz",
            ]),
            "x86_64",
            None,
            "acme/tool",
            "v1.2.3",
            Libc::Gnu,
        )
        .unwrap();

        match out {
            MatchOutput::NeedsInteraction { ranked, .. } => {
                assert_eq!(distinct_stems(&ranked), 2);
                let stems: Vec<&str> = ranked.iter().map(|r| r.stem()).collect();
                assert!(stems.contains(&"tool-cli"));
                assert!(stems.contains(&"tool-server"));
            }
            MatchOutput::AutoSelected(s) => panic!("expected a tie, got {}", s.asset().name),
        }
    }

    /// Competing builds of one binary must not look like separate binaries, or every
    /// ordinary release would start renaming itself.
    #[test]
    fn competing_builds_of_one_binary_are_not_variants() {
        let out = match_asset(
            assets(&[
                "ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz",
                "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz",
            ]),
            "x86_64",
            None,
            "BurntSushi/ripgrep",
            "14.1.0",
            Libc::Gnu,
        )
        .unwrap();

        match out {
            MatchOutput::AutoSelected(s) => {
                assert_eq!(s.ranked.stem(), "ripgrep");
                // One binary: naming stays the repo's business.
                assert_eq!(s.variant, None);
            }
            MatchOutput::NeedsInteraction { .. } => panic!("expected an auto-selection"),
        }
    }

    /// A nameless asset says nothing about how many binaries a release ships, and must
    /// never be counted into a variant split — naming a tool the empty string would
    /// otherwise follow.
    #[test]
    fn a_nameless_asset_is_not_counted_as_a_variant() {
        let out = match_asset(
            assets(&["linux-amd64"]),
            "x86_64",
            None,
            "github/gh-skyline",
            "v0.1.9",
            Libc::Gnu,
        )
        .unwrap();

        match out {
            MatchOutput::AutoSelected(s) => {
                assert_eq!(s.ranked.stem(), "");
                assert_eq!(s.variant, None);
            }
            MatchOutput::NeedsInteraction { .. } => panic!("expected an auto-selection"),
        }
    }

    #[test]
    fn a_single_best_candidate_is_auto_selected() {
        let out = match_asset(
            assets(&[
                "ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz",
                "ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz",
                "ripgrep-14.1.0-x86_64-pc-windows-msvc.zip",
            ]),
            "x86_64",
            None,
            "BurntSushi/ripgrep",
            "14.1.0",
            Libc::Gnu,
        )
        .unwrap();

        match out {
            MatchOutput::AutoSelected(s) => {
                assert_eq!(
                    s.asset().name,
                    "ripgrep-14.1.0-x86_64-unknown-linux-gnu.tar.gz"
                );
                assert!(s.notes.is_empty());
            }
            MatchOutput::NeedsInteraction { .. } => panic!("expected an auto-selection"),
        }
    }

    #[test]
    fn tied_candidates_ask_rather_than_guess() {
        let out = match_asset(
            assets(&[
                "tool-x86_64-linux-gnu.tar.gz",
                "tool-x86_64-linux-gnu-v3.tar.gz",
            ]),
            "x86_64",
            None,
            "acme/tool",
            "1.0",
            Libc::Gnu,
        )
        .unwrap();

        match out {
            MatchOutput::NeedsInteraction { ranked, .. } => assert_eq!(ranked.len(), 2),
            MatchOutput::AutoSelected(s) => panic!("expected a tie, got {}", s.asset().name),
        }
    }

    #[test]
    fn an_unmet_preference_still_installs_but_says_so() {
        let out = match_asset(
            assets(&["delta-0.17.0-x86_64-unknown-linux-musl.tar.gz"]),
            "x86_64",
            None,
            "dandavison/delta",
            "0.17.0",
            Libc::Gnu,
        )
        .unwrap();

        match out {
            MatchOutput::AutoSelected(s) => assert_eq!(
                s.notes,
                vec![SelectionNote::Fallback {
                    dimension: "libc",
                    wanted: "gnu",
                    got: "musl",
                }]
            ),
            MatchOutput::NeedsInteraction { .. } => panic!("expected an auto-selection"),
        }
    }

    #[test]
    fn nothing_installable_is_an_error() {
        let err = match_asset(
            assets(&[
                "tool_windows_amd64.zip",
                "tool-aarch64-unknown-linux-gnu.tar.gz",
                "checksums.txt",
            ]),
            "x86_64",
            None,
            "acme/tool",
            "1.0",
            Libc::Gnu,
        )
        .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<BintoError>(),
            Some(BintoError::NoCompatibleAssets { .. })
        ));
    }

    #[test]
    fn a_stored_pattern_pins_the_previous_choice() {
        let out = match_asset(
            assets(&[
                "tool-1.1.0-x86_64-linux-gnu.tar.gz",
                "tool-1.1.0-x86_64-linux-musl.tar.gz",
            ]),
            "x86_64",
            Some("tool-*-x86_64-linux-musl.tar.gz"),
            "acme/tool",
            "1.1.0",
            Libc::Gnu,
        )
        .unwrap();

        match out {
            // The pattern wins over the gnu preference — it is what the user installed
            // last time — but the unmet preference is still reported.
            MatchOutput::AutoSelected(s) => {
                assert_eq!(s.asset().name, "tool-1.1.0-x86_64-linux-musl.tar.gz");
                assert_eq!(
                    s.notes,
                    vec![SelectionNote::Fallback {
                        dimension: "libc",
                        wanted: "gnu",
                        got: "musl",
                    }]
                );
            }
            MatchOutput::NeedsInteraction { .. } => panic!("expected the pattern to pin one asset"),
        }
    }

    /// A pattern that survives into a release where it now matches something unusable
    /// must not short-circuit the pipeline.
    #[test]
    fn a_stored_pattern_matching_an_unusable_asset_falls_back() {
        let out = match_asset(
            assets(&[
                "tool-1.1.0-x86_64-linux.deb",
                "tool-1.1.0-x86_64-linux-gnu.tar.gz",
            ]),
            "x86_64",
            Some("tool-*-x86_64-linux.deb"),
            "acme/tool",
            "1.1.0",
            Libc::Gnu,
        )
        .unwrap();

        match out {
            MatchOutput::AutoSelected(s) => {
                assert_eq!(s.asset().name, "tool-1.1.0-x86_64-linux-gnu.tar.gz")
            }
            MatchOutput::NeedsInteraction { .. } => panic!("expected the fallback to auto-select"),
        }
    }
}
