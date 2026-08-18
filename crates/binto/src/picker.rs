use anyhow::Result;

use crate::config::Libc;
use crate::github::types::{Asset, Release};
use crate::matcher::rank::SelectionNote;
use crate::matcher::{MatchOutput, match_asset};
use crate::output::print_info;

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
) -> Result<Asset> {
    let match_output = match_asset(
        release.assets.clone(),
        user_arch,
        pattern,
        repo,
        &release.tag_name,
        prefer_libc,
    )?;

    let asset = match match_output {
        MatchOutput::AutoSelected(s) => {
            print_info(&format!("Auto-selected asset: {}", s.asset().name));
            // Say so when the release could not satisfy a preference, instead of letting
            // a fallback look identical to a match.
            for note in &s.notes {
                print_info(&format!("  ↳ {}", describe(note)));
            }
            s.ranked.candidate.asset
        }
        MatchOutput::NeedsInteraction {
            ranked: mut candidates,
            ..
        } => {
            if assume_yes {
                // Non-interactive: take the first of the tied candidates.
                let top = candidates.swap_remove(0).candidate.asset;
                print_info(&format!("Auto-selected asset (--yes): {}", top.name));
                top
            } else {
                let names: Vec<String> = candidates.iter().map(|c| c.name().to_string()).collect();
                let idx = dialoguer::Select::new()
                    .with_prompt(prompt)
                    .items(&names)
                    .default(0)
                    .interact()?;
                candidates.into_iter().nth(idx).unwrap().candidate.asset
            }
        }
    };

    Ok(asset)
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
