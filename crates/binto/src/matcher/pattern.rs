use glob::{MatchOptions, Pattern};

use super::facts::AssetName;

/// Generate a glob pattern from an asset name by replacing its version with `*`.
/// E.g.: "gh_2.45.0_linux_amd64.tar.gz" with tag "v2.45.0" → "gh_*_linux_amd64.tar.gz"
///
/// Only the version is replaced, found the way the stem finds it. Replacing the whole tag
/// took the binary's name with it whenever the tag carried one: `lutgen-studio-v0.4.0-...`
/// became `*-x86_64-unknown-linux-gnu`, which matches `lutgen-cli` too. Lowercased, like the
/// stem; [`match_pattern`] ignores case.
pub fn asset_to_pattern(asset_name: &str, tag: &str) -> String {
    let lower = asset_name.to_lowercase();
    match AssetName::new(asset_name).version_range(tag) {
        Some(span) => format!(
            "{}*{}",
            Pattern::escape(&lower[..span.start]),
            Pattern::escape(&lower[span.end..])
        ),
        // No version to replace — the exact name is the pattern.
        None => Pattern::escape(&lower),
    }
}

/// Try to match a list of asset names against a stored glob pattern.
/// Returns matching asset names.
pub fn match_pattern<'a>(pattern: &str, asset_names: &[&'a str]) -> Vec<&'a str> {
    let Ok(pat) = Pattern::new(pattern) else {
        tracing::debug!(pattern, "stored pattern is not a valid glob");
        return vec![];
    };
    let matched: Vec<&str> = asset_names
        .iter()
        // `glob` folds only ASCII case; lowercasing the name covers a pattern written
        // lowercase for an asset with a non-ASCII capital.
        .filter(|name| {
            pat.matches_with(
                name,
                MatchOptions {
                    case_sensitive: false,
                    ..MatchOptions::new()
                },
            ) || pat.matches(&name.to_lowercase())
        })
        .copied()
        .collect();
    tracing::debug!(
        pattern,
        match_count = matched.len(),
        "matched stored pattern"
    );
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_pattern_strip_v() {
        let p = asset_to_pattern("gh_2.45.0_linux_amd64.tar.gz", "v2.45.0");
        assert_eq!(p, "gh_*_linux_amd64.tar.gz");
    }

    #[test]
    fn generates_pattern_tag_with_v_in_name() {
        let p = asset_to_pattern("tool-v1.2.3-linux-amd64", "v1.2.3");
        assert_eq!(p, "tool-*-linux-amd64");
    }

    #[test]
    fn pattern_matches_new_version() {
        let p = asset_to_pattern("gh_2.45.0_linux_amd64.tar.gz", "v2.45.0");
        let names = vec![
            "gh_2.50.0_linux_amd64.tar.gz",
            "gh_2.50.0_linux_arm64.tar.gz",
        ];
        let matched = match_pattern(&p, &names);
        assert_eq!(matched, vec!["gh_2.50.0_linux_amd64.tar.gz"]);
    }

    /// A tag naming the binary must not take the name out of the pattern with it, or the
    /// pattern matches every binary the release ships.
    #[test]
    fn a_name_prefixed_tag_keeps_the_name_in_the_pattern() {
        let p = asset_to_pattern(
            "lutgen-studio-v0.4.0-x86_64-unknown-linux-gnu",
            "lutgen-studio-v0.4.0",
        );
        assert_eq!(p, "lutgen-studio-*-x86_64-unknown-linux-gnu");
    }

    /// A sibling binary's version is not in the tag, and a pattern keeping it literally
    /// matches nothing in the next release.
    #[test]
    fn a_version_absent_from_the_tag_is_still_replaced() {
        let p = asset_to_pattern(
            "lutgen-cli-v1.1.1-x86_64-unknown-linux-gnu",
            "lutgen-studio-v0.4.0",
        );
        assert_eq!(p, "lutgen-cli-*-x86_64-unknown-linux-gnu");
        let next = [
            "lutgen-cli-v1.2.0-x86_64-unknown-linux-gnu",
            "lutgen-studio-v0.4.0-x86_64-unknown-linux-gnu",
        ];
        assert_eq!(
            match_pattern(&p, &next),
            vec!["lutgen-cli-v1.2.0-x86_64-unknown-linux-gnu"]
        );
    }

    /// Patterns written before they were lowercased still match.
    #[test]
    fn matching_ignores_case() {
        let names = ["Tool_1.2.0_Linux_x86_64.tar.gz"];
        assert_eq!(match_pattern("Tool_*_Linux_x86_64.tar.gz", &names), names);
        assert_eq!(match_pattern("tool_*_linux_x86_64.tar.gz", &names), names);
    }
}
