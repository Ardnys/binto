// TODO: this file is a bit redundant. could be moved to matcher::mod.rs
use anyhow::Result;

use crate::config::Libc;
use crate::github::types::{Asset, Release};
use crate::installer::default_binary_name;
use crate::matcher::rank::SelectionNote;
use crate::matcher::{MatchOutput, distinct_stems, match_asset};
use crate::output::{print_info, print_warning};

// TODO: this could be an impl SelectionNote
/// One line explaining what the release could not give you, for the notes the matcher
/// attached to its pick.
fn describe(note: &SelectionNote) -> String {
    match note {
        SelectionNote::Fallback {
            dimension,
            wanted,
            got,
        } => format!("{dimension}: {got} (preferred {wanted} not available)"),
        SelectionNote::Unspecified { dimension } => {
            format!("{dimension}: not stated by the asset name")
        }
    }
}

/// One asset, chosen, plus which of the repo's binaries it turned out to be.
pub struct SelectedAsset {
    pub asset: Asset,
    /// The stem to install under, set only when the release ships several distinct
    /// binaries. `None` means the repo-derived name is correct — see [`Selection::variant`].
    pub variant: Option<String>,
}

/// Resolve a release to a single concrete asset for the current arch.
///
/// Auto-selects when the matcher is confident; otherwise falls back to an interactive
/// picker (or, when `assume_yes`, the top-scored candidate). `pattern` is the tool's stored
/// `asset_pattern` for updates, or `None` for a fresh install. This is the single selection path
/// shared by install and both update flows.
pub fn select_asset(
    release: &Release,
    user_arch: &str,
    pattern: Option<&str>,
    repo: &str,
    prompt: &str,
    prefer_libc: Libc,
    assume_yes: bool,
) -> Result<SelectedAsset> {
    let match_output = match_asset(
        release.assets.clone(),
        user_arch,
        pattern,
        repo,
        &release.tag_name,
        prefer_libc,
    )?;

    let selected = match match_output {
        MatchOutput::AutoSelected(s) => {
            print_info(&format!("Auto-selected asset: {}", s.asset().name));
            // Say so when the release could not satisfy a preference, instead of letting
            // a fallback look identical to a match.
            for note in &s.notes {
                print_info(&format!("  ↳ {}", describe(note)));
            }
            SelectedAsset {
                variant: s.variant,
                asset: s.ranked.candidate.asset,
            }
        }
        MatchOutput::NeedsInteraction {
            ranked: mut candidates,
            ..
        } => {
            // Whether the release ships several binaries is a property of the release, so
            // it is settled before the user narrows it down to one.
            let ships_variants = distinct_stems(&candidates) > 1;

            let primary = default_binary_name(repo);

            let chosen = if assume_yes {
                // Nobody to ask, and tied candidates are in whatever order the release
                // listed them. Prefer the binary named after the repo, which is as close to
                // a "primary" as a release gets.
                let idx = candidates
                    .iter()
                    .position(|c| c.stem() == primary)
                    .unwrap_or(0);
                let top = candidates.swap_remove(idx);
                print_info(&format!("Auto-selected asset (--yes): {}", top.name()));
                top
            } else {
                let names: Vec<String> = candidates.iter().map(|c| c.name().to_string()).collect();
                let idx = dialoguer::Select::new()
                    .with_prompt(prompt)
                    .items(&names)
                    .default(0)
                    .interact()?;
                candidates.into_iter().nth(idx).unwrap()
            };

            // Naming by stem is only safe when a person saw which asset they picked. Under
            // `--yes` an arbitrary tied asset would otherwise name the tool after itself —
            // installing `oatmeal` as `debug`.
            let named_by_stem = ships_variants
                && !chosen.stem().is_empty()
                && (!assume_yes || chosen.stem() == primary);

            if assume_yes && ships_variants && chosen.stem() != primary {
                print_warning(&format!(
                    "{repo} ships several binaries and --yes cannot ask which one you want. \
                     Installing {} as '{primary}'.",
                    chosen.name()
                ));
            }

            SelectedAsset {
                variant: named_by_stem.then(|| chosen.stem().to_string()),
                asset: chosen.candidate.asset,
            }
        }
    };

    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unmet_libc_preference_reads_as_a_fallback() {
        assert_eq!(
            describe(&SelectionNote::Fallback {
                dimension: "libc",
                wanted: "gnu",
                got: "musl",
            }),
            "libc: musl (preferred gnu not available)"
        );
    }

    #[test]
    fn an_unstated_dimension_says_so() {
        assert_eq!(
            describe(&SelectionNote::Unspecified { dimension: "arch" }),
            "arch: not stated by the asset name"
        );
    }
}
