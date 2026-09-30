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
use facts::AssetName;
use filter::{Candidate, apply_hard_filters};
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

/// What a previous install chose, for an update to choose the same again.
#[derive(Debug, Clone, Copy)]
pub struct PreviousChoice<'a> {
    /// The installed asset's name with its version replaced by `*`.
    pub pattern: &'a str,
    /// Which of the repo's binaries was installed. Empty when nothing was recorded.
    pub stem: &'a str,
}

/// Main entry point for asset matching.
///
/// If a `previous` choice is provided (from a previous install), try the pattern fast-path
/// first. If the pattern matches zero or several assets, rank the full pipeline — but only
/// among assets of the previously installed binary, when the release still ships it. A
/// repo tagging per product otherwise ties `lutgen-cli` against `lutgen-studio` on every
/// update, though the tool is already one of them.
#[tracing::instrument(
    skip_all,
    fields(repo = repo, tag = tag, arch = user_arch, libc = ?prefer_libc)
)]
pub fn match_asset(
    all_assets: Vec<Asset>,
    user_arch: &str,
    previous: Option<PreviousChoice>,
    repo: &str,
    tag: &str,
    prefer_libc: Libc,
) -> Result<MatchOutput> {
    let profile = PreferenceProfile::new(prefer_libc);

    // Pattern fast-path: if we have a stored pattern and it matches exactly one asset.
    if let Some(prev) = previous
        && let Some(selection) = pattern_fast_path(prev, &all_assets, user_arch, tag, &profile)
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
    let candidates = match previous {
        Some(prev) if !prev.stem.is_empty() => narrow_to_stem(candidates, prev.stem),
        _ => candidates,
    };

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

    // WARN: there's a slightly aggressive filtering here. Perhaps add an option?
    // When there's multiple binary variants, only show tied assets.
    // This reduces the clutter in some projects, as GNU/MUSL variants
    // survive the filtering but are not tied in the ranks.
    // However, there are false negatives when there are many options of binaries
    // with various naming schemes, and they might not be tied. Then, this silently
    // removes them from the selection screen.
    if ships_variants && ranked.len() != tied {
        // trim the ranked to tied to get rid of lower ranked ones
        // like unpreferred libc variants
        let runners_up: Vec<RankedAsset> = ranked.into_iter().take(tied).collect();
        let notes = notes_for(&runners_up[0], &profile);
        trace_selection("needs_interaction", &runners_up[0], tied, &notes);
        return Ok(MatchOutput::NeedsInteraction {
            ranked: runners_up,
            notes,
        });
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

/// Only the candidates of the binary a previous install chose, unless the release no longer
/// ships it (or the stem was recorded by an older, buggier stemmer) — then all of them, and
/// the tie that follows lets the user pick again.
fn narrow_to_stem(candidates: Vec<Candidate>, stem: &str) -> Vec<Candidate> {
    if !candidates.iter().any(|c| c.stem == stem) {
        debug!(
            stem,
            "no candidate carries the installed stem, ranking all of them"
        );
        return candidates;
    }
    let before = candidates.len();
    let kept: Vec<Candidate> = candidates.into_iter().filter(|c| c.stem == stem).collect();
    debug!(
        stem,
        before,
        after = kept.len(),
        "narrowed to the installed stem"
    );
    kept
}

/// The stem to name an asset by, or `None` to leave naming to the repo.
fn variant_of(chosen: &RankedAsset, ships_variants: bool) -> Option<String> {
    (ships_variants && !chosen.stem().is_empty()).then(|| chosen.stem().to_string())
}

/// Re-select the asset a previous install chose. Returns `None` when the pattern does not
/// pin exactly one asset of the installed binary, or when that asset would not survive the
/// hard filters — a stored pattern is a shortcut, never a licence to install something
/// unusable.
fn pattern_fast_path(
    prev: PreviousChoice,
    all_assets: &[Asset],
    user_arch: &str,
    tag: &str,
    profile: &PreferenceProfile,
) -> Option<Selection> {
    let pat = prev.pattern;
    let names: Vec<&str> = all_assets.iter().map(|a| a.name.as_str()).collect();
    let mut matched = pattern::match_pattern(pat, &names);
    // A `*` spans separators, so `tool-*-linux` also matches `tool-server-1.2-linux`. The
    // stem says which of those the tool is. Only a tie is narrowed, and never to nothing: a
    // stem recorded by an older stemmer (`lutgen-cli-v1.1.1`) matches no asset today, and
    // must not veto the one asset its pattern still pins.
    if matched.len() > 1 && !prev.stem.is_empty() {
        let same_stem: Vec<&str> = matched
            .iter()
            .copied()
            .filter(|name| AssetName::new(*name).stem(Some(tag)) == prev.stem)
            .collect();
        if !same_stem.is_empty() {
            matched = same_stem;
        }
    }

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
            Some(PreviousChoice {
                pattern: "tool-*-x86_64-linux-musl.tar.gz",
                stem: "tool",
            }),
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
            Some(PreviousChoice {
                pattern: "tool-*-x86_64-linux.deb",
                stem: "tool",
            }),
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

    /// The release `ozwaldorf/lutgen-rs` tagged for its studio, carrying the CLI's latest
    /// build too. Both tie on every tier.
    fn lutgen_studio_release() -> Vec<Asset> {
        assets(&[
            "lutgen-cli-v1.1.1-x86_64-unknown-linux-gnu",
            "lutgen-studio-v0.4.0-x86_64-unknown-linux-gnu",
        ])
    }

    fn update_lutgen(pattern: &str, stem: &str) -> MatchOutput {
        match_asset(
            lutgen_studio_release(),
            "x86_64",
            Some(PreviousChoice { pattern, stem }),
            "ozwaldorf/lutgen-rs",
            "lutgen-studio-v0.4.0",
            Libc::Gnu,
        )
        .unwrap()
    }

    fn selected(out: MatchOutput) -> String {
        match out {
            MatchOutput::AutoSelected(s) => s.asset().name.clone(),
            MatchOutput::NeedsInteraction { ranked, .. } => {
                let names: Vec<&str> = ranked.iter().map(|r| r.name()).collect();
                panic!("expected an auto-selection, got a tie between {names:?}")
            }
        }
    }

    /// Both entries of a real state file written before this fix. The studio's pattern
    /// lost its name to the tag and matches both assets; the CLI's kept its version and
    /// matches neither. Each tool still knows which binary it is.
    #[test]
    fn an_update_reselects_the_installed_binary_when_its_pattern_is_inconclusive() {
        assert_eq!(
            selected(update_lutgen("*-x86_64-unknown-linux-gnu", "lutgen-studio")),
            "lutgen-studio-v0.4.0-x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            selected(update_lutgen(
                "lutgen-cli-v1.1.1-x86_64-unknown-linux-gnu",
                "lutgen-cli"
            )),
            "lutgen-cli-v1.1.1-x86_64-unknown-linux-gnu"
        );
    }

    /// A `*` spans separators, so a pattern for `tool` also matches `tool-server`; the stem
    /// settles it without falling back to ranking.
    #[test]
    fn the_stem_disambiguates_a_pattern_matching_several_binaries() {
        let out = match_asset(
            assets(&[
                "tool-server-1.1.0-x86_64-linux-gnu.tar.gz",
                "tool-1.1.0-x86_64-linux-gnu.tar.gz",
            ]),
            "x86_64",
            Some(PreviousChoice {
                pattern: "tool-*-x86_64-linux-gnu.tar.gz",
                stem: "tool",
            }),
            "acme/tool",
            "1.1.0",
            Libc::Gnu,
        )
        .unwrap();
        assert_eq!(selected(out), "tool-1.1.0-x86_64-linux-gnu.tar.gz");
    }

    /// A stem recorded by an older stemmer matches nothing now. That must not lock the tool
    /// out of updates — it falls back to the choice a fresh install would offer.
    #[test]
    fn an_unrecognised_stem_falls_back_to_every_candidate() {
        match update_lutgen("no-match", "lutgen-cli-v1.1.1") {
            MatchOutput::NeedsInteraction { ranked, .. } => assert_eq!(ranked.len(), 2),
            MatchOutput::AutoSelected(s) => panic!("expected a tie, got {}", s.asset().name),
        }
    }

    /// State written before this fix: the stem kept its version, but the literal pattern
    /// still pins the asset while the release keeps shipping that exact build.
    #[test]
    fn a_stale_stem_does_not_veto_a_pattern_that_pins_one_asset() {
        assert_eq!(
            selected(update_lutgen(
                "lutgen-cli-v1.1.1-x86_64-unknown-linux-gnu",
                "lutgen-cli-v1.1.1"
            )),
            "lutgen-cli-v1.1.1-x86_64-unknown-linux-gnu"
        );
    }
}
