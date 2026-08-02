/// Generate a glob pattern from an asset name by replacing the version string with `*`.
/// E.g.: "gh_2.45.0_linux_amd64.tar.gz" with tag "v2.45.0" → "gh_*_linux_amd64.tar.gz"
pub fn asset_to_pattern(asset_name: &str, tag: &str) -> String {
    let version = tag.trim_start_matches('v');
    let with_v = asset_name.replace(tag, "*");
    if with_v != asset_name {
        return with_v;
    }
    let without_v = asset_name.replace(version, "*");
    if without_v != asset_name {
        return without_v;
    }
    // No substitution possible — return the exact name as the pattern
    asset_name.to_string()
}

/// Try to match a list of asset names against a stored glob pattern.
/// Returns matching asset names.
pub fn match_pattern<'a>(pattern: &str, asset_names: &[&'a str]) -> Vec<&'a str> {
    let Ok(pat) = glob::Pattern::new(pattern) else {
        tracing::debug!(pattern, "stored pattern is not a valid glob");
        return vec![];
    };
    let matched: Vec<&str> = asset_names
        .iter()
        .filter(|name| pat.matches(name))
        .copied()
        .collect();
    tracing::debug!(pattern, match_count = matched.len(), "matched stored pattern");
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
}
