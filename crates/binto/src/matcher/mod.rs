pub mod filter;
pub mod pattern;
pub mod score;

use anyhow::Result;
use tracing::debug;

use crate::config::Libc;
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
            let score = score::score_asset(&asset, user_arch, prefer_libc);
            debug!(
                pattern = pat,
                asset = %asset.name,
                total = score.total,
                "pattern fast-path selected asset"
            );
            return Ok(MatchOutput::AutoSelected(ScoredAsset { asset, score }));
        }
        debug!(
            pattern = pat,
            match_count = matched.len(),
            "pattern fast-path inconclusive, falling back to scoring"
        );
    }

    // Apply hard filters
    let total_assets = all_assets.len();
    let candidates = apply_hard_filters(all_assets);
    debug!(
        before = total_assets,
        after = candidates.len(),
        "applied hard filters"
    );

    if candidates.is_empty() {
        debug!("no candidates left after hard filters");
        return Err(BintoError::NoCompatibleAssets {
            repo: repo.to_string(),
            tag: tag.to_string(),
        }
        .into());
    }

    // Score and rank
    let scored = score_and_rank(candidates, user_arch, prefer_libc);

    if scored.is_empty() {
        debug!("no candidates left after scoring");
        return Err(BintoError::NoCompatibleAssets {
            repo: repo.to_string(),
            tag: tag.to_string(),
        }
        .into());
    }

    // Confidence check
    if scored.len() == 1 {
        debug!(
            asset = %scored[0].asset.name,
            total = scored[0].score.total,
            reason = "single_candidate",
            "auto-selected asset"
        );
        return Ok(MatchOutput::AutoSelected(
            scored.into_iter().next().unwrap(),
        ));
    }

    let gap = scored[0].score.total - scored[1].score.total;
    if gap >= CONFIDENCE_THRESHOLD {
        debug!(
            asset = %scored[0].asset.name,
            total = scored[0].score.total,
            runner_up = %scored[1].asset.name,
            runner_up_total = scored[1].score.total,
            gap,
            threshold = CONFIDENCE_THRESHOLD,
            reason = "confidence_gap",
            "auto-selected asset"
        );
        Ok(MatchOutput::AutoSelected(
            scored.into_iter().next().unwrap(),
        ))
    } else {
        debug!(
            top = %scored[0].asset.name,
            total = scored[0].score.total,
            runner_up = %scored[1].asset.name,
            runner_up_total = scored[1].score.total,
            gap,
            threshold = CONFIDENCE_THRESHOLD,
            "confidence gap below threshold, needs interaction"
        );
        Ok(MatchOutput::NeedsInteraction(scored))
    }
}
