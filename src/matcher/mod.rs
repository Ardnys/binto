pub mod filter;
pub mod pattern;
pub mod score;

use anyhow::Result;

use crate::error::BintoError;
use crate::github::types::Asset;
use filter::apply_hard_filters;
use score::{CONFIDENCE_THRESHOLD, ScoredAsset, score_and_rank};

pub enum MatchOutput {
    AutoSelected(ScoredAsset),
    NeedsInteraction(Vec<ScoredAsset>),
}

/// Main entry point for asset matching.
///
/// If `stored_pattern` is provided (from a previous install), try the pattern fast-path
/// first. Falls back to full scoring if the pattern matches zero or multiple assets.
pub fn match_asset(
    all_assets: Vec<Asset>,
    user_arch: &str,
    stored_pattern: Option<&str>,
    repo: &str,
    tag: &str,
) -> Result<MatchOutput> {
    // Pattern fast-path: if we have a stored pattern and it matches exactly one asset
    if let Some(pat) = stored_pattern {
        let names: Vec<String> = all_assets.iter().map(|a| a.name.clone()).collect();
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let matched = pattern::match_pattern(pat, &name_refs);
        if matched.len() == 1 {
            let matched_name = matched[0].to_string();
            let asset = all_assets
                .into_iter()
                .find(|a| a.name == matched_name)
                .unwrap();
            let score = score::score_asset(&asset, user_arch);
            return Ok(MatchOutput::AutoSelected(ScoredAsset { asset, score }));
        }
    }

    // Apply hard filters
    let candidates = apply_hard_filters(all_assets);

    if candidates.is_empty() {
        return Err(BintoError::NoCompatibleAssets {
            repo: repo.to_string(),
            tag: tag.to_string(),
        }
        .into());
    }

    // Score and rank
    let scored = score_and_rank(candidates, user_arch);

    if scored.is_empty() {
        return Err(BintoError::NoCompatibleAssets {
            repo: repo.to_string(),
            tag: tag.to_string(),
        }
        .into());
    }

    // Confidence check
    if scored.len() == 1 {
        return Ok(MatchOutput::AutoSelected(
            scored.into_iter().next().unwrap(),
        ));
    }

    let gap = scored[0].score.total - scored[1].score.total;
    if gap >= CONFIDENCE_THRESHOLD {
        Ok(MatchOutput::AutoSelected(
            scored.into_iter().next().unwrap(),
        ))
    } else {
        Ok(MatchOutput::NeedsInteraction(scored))
    }
}
