//! Marked repositories, written out in the dataset's own shape.
//!
//! Each line is a valid dataset line — `repo`, `tag`, and `assets` each with a `name` — so a
//! marked file goes straight back through `runner --dataset` after a fix, and any single line
//! can be piped into `binto match`. What the matcher decided rides along as extra fields,
//! which both of those ignore.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use serde::Serialize;

use crate::model::{Outcome, RepoView};

/// Write one JSON line per repo. Returns how many were written.
pub fn write<'a>(path: &Path, repos: impl IntoIterator<Item = &'a RepoView>) -> io::Result<usize> {
    let mut out = BufWriter::new(File::create(path)?);
    let mut count = 0;
    for repo in repos {
        serde_json::to_writer(&mut out, &ExportLine::from(repo)).map_err(io::Error::other)?;
        out.write_all(b"\n")?;
        count += 1;
    }
    out.flush()?;
    Ok(count)
}

// A struct rather than `json!` so fields keep this order — `repo` first, `assets` last — which
// is what makes a line readable by eye.
#[derive(Serialize)]
struct ExportLine<'a> {
    repo: &'a str,
    tag: &'a str,
    outcome: &'static str,
    html_url: String,
    release_url: String,
    stems: &'a [String],
    checksum: Option<&'a str>,
    /// Survivors in rank order, then rejections.
    assets: Vec<ExportAsset<'a>>,
}

#[derive(Serialize)]
struct ExportAsset<'a> {
    name: &'a str,
    /// `selected`, `tied`, `ranked`, or `rejected`.
    fate: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rank: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stem: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tiers: Option<Tiers<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    marker: Option<&'a str>,
}

#[derive(Serialize)]
struct Tiers<'a> {
    arch: &'a str,
    os: &'a str,
    libc: &'a str,
    format: &'a str,
}

impl<'a> From<&'a RepoView> for ExportLine<'a> {
    fn from(repo: &'a RepoView) -> Self {
        let ranked = repo.survivors.iter().enumerate().map(|(i, s)| ExportAsset {
            name: &s.name,
            fate: if s.selected {
                "selected"
            } else if s.tied {
                "tied"
            } else {
                "ranked"
            },
            rank: Some(i + 1),
            stem: s.stem.as_deref(),
            tiers: Some(Tiers {
                arch: &s.dims[0].label,
                os: &s.dims[1].label,
                libc: &s.dims[2].label,
                format: &s.dims[3].label,
            }),
            reason: None,
            marker: None,
        });
        let rejected = repo.rejected.iter().map(|r| ExportAsset {
            name: &r.name,
            fate: "rejected",
            rank: None,
            stem: None,
            tiers: None,
            reason: Some(&r.reason),
            marker: Some(&r.marker),
        });

        ExportLine {
            repo: &repo.repo,
            tag: &repo.tag,
            outcome: match repo.outcome {
                Outcome::AutoSelected => "auto_selected",
                Outcome::NeedsInteraction => "needs_interaction",
                Outcome::NoMatch => "no_match",
                Outcome::Error => "error",
            },
            html_url: format!("https://github.com/{}", repo.repo),
            release_url: format!("https://github.com/{}/releases/tag/{}", repo.repo, repo.tag),
            stems: &repo.stems,
            checksum: repo.checksum.as_deref(),
            assets: ranked.chain(rejected).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dim, Rejected, Survivor};

    fn repo() -> RepoView {
        let dim = |label: &str| Dim {
            label: label.to_string(),
            tier: Some(0),
        };
        RepoView {
            repo: "ozwaldorf/lutgen-rs".to_string(),
            tag: "lutgen-studio-v0.4.0".to_string(),
            outcome: Outcome::NeedsInteraction,
            n_assets: 3,
            survivors: vec![Survivor {
                name: "lutgen-cli-v1.1.1-x86_64-unknown-linux-gnu".to_string(),
                stem: Some("lutgen-cli-v1.1.1".to_string()),
                dims: [dim("x86_64"), dim("linux"), dim("gnu"), dim("raw")],
                notes: vec![],
                selected: false,
                tied: true,
            }],
            rejected: vec![Rejected {
                name: "lutgen-studio-v0.4.0-x86_64-pc-windows-msvc.exe".to_string(),
                reason: "foreign_os".to_string(),
                marker: "windows".to_string(),
            }],
            checksum: None,
            stems: vec!["lutgen-cli-v1.1.1".to_string()],
            error: None,
            raw_trace: vec![],
        }
    }

    /// The point of the format: an exported line is still a release the harness can run.
    #[test]
    fn an_exported_line_is_a_release_binto_can_match_again() {
        let raw = serde_json::to_string(&ExportLine::from(&repo())).unwrap();
        let input: binto_contract::MatchInput = serde_json::from_str(&raw).unwrap();

        assert_eq!(input.tag.as_deref(), Some("lutgen-studio-v0.4.0"));
        let names: Vec<&str> = input.assets.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "lutgen-cli-v1.1.1-x86_64-unknown-linux-gnu",
                "lutgen-studio-v0.4.0-x86_64-pc-windows-msvc.exe"
            ]
        );
    }

    #[test]
    fn a_line_leads_with_the_repo_and_records_each_assets_fate() {
        let raw = serde_json::to_string(&ExportLine::from(&repo())).unwrap();

        assert!(raw.starts_with(r#"{"repo":"ozwaldorf/lutgen-rs","tag":"#));
        assert!(raw.contains(r#""release_url":"https://github.com/ozwaldorf/lutgen-rs/releases/tag/lutgen-studio-v0.4.0""#));
        assert!(raw.contains(r#""fate":"tied","rank":1"#));
        assert!(raw.contains(r#""fate":"rejected","reason":"foreign_os","marker":"windows""#));
    }

    #[test]
    fn write_puts_one_repo_on_each_line() {
        let path = std::env::temp_dir().join(format!("insite-export-{}.jsonl", std::process::id()));
        let (a, b) = (repo(), repo());

        let count = write(&path, [&a, &b]).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(count, 2);
        assert_eq!(written.lines().count(), 2);
    }
}
