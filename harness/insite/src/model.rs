//! The runner's records reshaped into what a person inspecting a release wants to see: which
//! assets it had, which the hard filters threw out and why, and how the rest were ranked.
//!
//! Built once at load. The trace is the source of truth for survivors, because an
//! auto-selected verdict lists only the winner — the losing candidates exist only as
//! `asset ranked` events.

use std::collections::HashMap;

use binto_contract::{Candidate, RunRecord, SelectionNote, TraceLine, messages};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    AutoSelected,
    NeedsInteraction,
    NoMatch,
    /// binto crashed or printed no verdict the runner could parse.
    Error,
}

impl Outcome {
    pub const ALL: [Outcome; 4] = [
        Outcome::AutoSelected,
        Outcome::NeedsInteraction,
        Outcome::NoMatch,
        Outcome::Error,
    ];

    fn from_label(label: &str) -> Self {
        match label {
            "auto_selected" => Outcome::AutoSelected,
            "needs_interaction" => Outcome::NeedsInteraction,
            "no_match" => Outcome::NoMatch,
            _ => Outcome::Error,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Outcome::AutoSelected => "auto",
            Outcome::NeedsInteraction => "interaction",
            Outcome::NoMatch => "no match",
            Outcome::Error => "error",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Outcome::AutoSelected => "●",
            Outcome::NeedsInteraction => "◐",
            Outcome::NoMatch => "○",
            Outcome::Error => "✕",
        }
    }
}

/// One preference dimension of a survivor: what the asset name said, and the tier it earned.
#[derive(Debug, Clone)]
pub struct Dim {
    pub label: String,
    /// `0` is best. `None` when the trace did not report it.
    pub tier: Option<i64>,
}

/// An asset that cleared the hard filters and was ranked.
#[derive(Debug)]
pub struct Survivor {
    pub name: String,
    /// `None` when binto did not report one. The verdict carries stems only for the
    /// candidates it lists, which for an auto-selection is the winner alone.
    pub stem: Option<String>,
    /// arch, os, libc, format — in the priority order the matcher compares them.
    pub dims: [Dim; 4],
    pub notes: Vec<String>,
    pub selected: bool,
    /// Part of the leading tie group that made binto ask instead of choose.
    pub tied: bool,
}

/// An asset a hard filter disqualified.
#[derive(Debug)]
pub struct Rejected {
    pub name: String,
    pub reason: String,
    pub marker: String,
}

#[derive(Debug)]
pub struct RepoView {
    pub repo: String,
    pub tag: String,
    pub outcome: Outcome,
    pub n_assets: usize,
    /// Best first.
    pub survivors: Vec<Survivor>,
    pub rejected: Vec<Rejected>,
    pub checksum: Option<String>,
    /// Distinct non-empty stems among survivors. More than one means the release ships
    /// several binaries — as far as binto could tell from what it reported.
    pub stems: Vec<String>,
    pub error: Option<String>,
    /// stderr lines that were not binto's JSON log, such as a panic.
    pub raw_trace: Vec<String>,
}

const DIMS: [&str; 4] = ["arch", "os", "libc", "format"];

impl RepoView {
    pub fn from_record(record: RunRecord) -> Self {
        let RunRecord {
            repo,
            tag,
            outcome,
            n_assets,
            verdict,
            trace,
            error,
            ..
        } = record;
        let outcome = Outcome::from_label(&outcome);

        let mut ranked = Vec::new();
        let mut rejected = Vec::new();
        let mut raw_trace = Vec::new();
        let mut tied = 0usize;

        for line in &trace {
            match line {
                TraceLine::Raw { raw } => raw_trace.push(raw.clone()),
                TraceLine::Event(event) => match event.message.as_str() {
                    messages::ASSET_RANKED => ranked.push(event),
                    messages::ASSET_REJECTED => rejected.push(Rejected {
                        name: event.field_str("asset").unwrap_or("?").to_string(),
                        reason: event.field_str("reason").unwrap_or("?").to_string(),
                        marker: event.field_str("marker").unwrap_or("").to_string(),
                    }),
                    messages::SELECTION => {
                        tied = event.field_i64("tied").unwrap_or(0).max(0) as usize;
                    }
                    _ => {}
                },
            }
        }

        let listed: HashMap<&str, &Candidate> = verdict
            .iter()
            .flat_map(|v| &v.candidates)
            .map(|c| (c.name.as_str(), c))
            .collect();
        let selected = verdict
            .as_ref()
            .and_then(|v| v.selected.as_ref())
            .map(|c| c.name.as_str());

        let survivors: Vec<Survivor> = if ranked.is_empty() {
            // No ranking trace — the stored-pattern fast path skips it — so the verdict's
            // candidate list is all there is.
            verdict
                .iter()
                .flat_map(|v| &v.candidates)
                .enumerate()
                .map(|(i, c)| Survivor {
                    stem: Some(c.stem.clone()),
                    dims: [
                        dim(&c.tiers.arch, None),
                        dim(&c.tiers.os, None),
                        dim(&c.tiers.libc, None),
                        dim(&c.tiers.format, None),
                    ],
                    notes: describe_all(&c.notes),
                    selected: selected == Some(c.name.as_str()),
                    tied: outcome == Outcome::NeedsInteraction && i < tied,
                    name: c.name.clone(),
                })
                .collect()
        } else {
            ranked
                .iter()
                .enumerate()
                .map(|(i, event)| {
                    let name = event.field_str("asset").unwrap_or("?").to_string();
                    let known = listed.get(name.as_str());
                    Survivor {
                        stem: known.map(|c| c.stem.clone()),
                        dims: DIMS.map(|d| {
                            dim(
                                event.field_str(d).unwrap_or("?"),
                                event.field_i64(&format!("{d}_tier")),
                            )
                        }),
                        notes: known.map(|c| describe_all(&c.notes)).unwrap_or_default(),
                        selected: selected == Some(name.as_str()),
                        tied: outcome == Outcome::NeedsInteraction && i < tied,
                        name,
                    }
                })
                .collect()
        };

        let mut stems: Vec<String> = survivors
            .iter()
            .filter_map(|s| s.stem.clone())
            .filter(|s| !s.is_empty())
            .collect();
        stems.sort();
        stems.dedup();

        let tag = tag
            .or_else(|| verdict.as_ref().map(|v| v.tag.clone()))
            .unwrap_or_default();
        let checksum = verdict.and_then(|v| v.checksum);

        RepoView {
            repo,
            tag,
            outcome,
            n_assets,
            survivors,
            rejected,
            checksum,
            stems,
            error,
            raw_trace,
        }
    }

    /// Case-insensitive substring match on the repo name or any asset name. `query` must
    /// already be lowercase.
    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.repo.to_lowercase().contains(query)
            || self
                .survivors
                .iter()
                .any(|s| s.name.to_lowercase().contains(query))
            || self
                .rejected
                .iter()
                .any(|r| r.name.to_lowercase().contains(query))
    }
}

fn dim(label: &str, tier: Option<i64>) -> Dim {
    Dim {
        label: label.to_string(),
        tier,
    }
}

/// One line per unmet preference, with every unstated dimension folded into a single line —
/// a leader that names nothing would otherwise spend three rows saying so.
fn describe_all(notes: &[SelectionNote]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut unstated = Vec::new();
    for note in notes {
        match note {
            SelectionNote::Fallback {
                dimension,
                wanted,
                got,
            } => lines.push(format!("{dimension}: got {got}, wanted {wanted}")),
            SelectionNote::Unspecified { dimension } => unstated.push(dimension.as_str()),
        }
    }
    if !unstated.is_empty() {
        lines.push(format!("not stated by the name: {}", unstated.join(", ")));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(json: &str) -> RepoView {
        RepoView::from_record(serde_json::from_str(json).unwrap())
    }

    fn event(message: &str, fields: &str) -> String {
        format!(
            r#"{{"timestamp":"t","level":"DEBUG","message":"{message}","target":"binto"{fields}}}"#
        )
    }

    fn ranked(asset: &str, libc: &str, libc_tier: i64) -> String {
        event(
            "asset ranked",
            &format!(
                r#","asset":"{asset}","arch":"x86_64","os":"linux","libc":"{libc}","format":"tar","arch_tier":0,"os_tier":0,"libc_tier":{libc_tier},"format_tier":0"#
            ),
        )
    }

    fn candidate(name: &str, stem: &str) -> String {
        format!(
            r#"{{"name":"{name}","stem":"{stem}","tiers":{{"arch":"x86_64","os":"linux","libc":"gnu","format":"tar"}},"notes":[]}}"#
        )
    }

    #[test]
    fn a_tie_marks_the_leading_group_and_collects_every_stem() {
        let cli = "tool-cli-1.2.3-x86_64-linux-gnu.tar.gz";
        let server = "tool-server-1.2.3-x86_64-linux-gnu.tar.gz";
        let view = record(&format!(
            r#"{{"repo":"acme/tool","tag":"v1.2.3","arch":"x86_64","libc":"gnu","n_assets":3,
               "outcome":"needs_interaction","exit_code":42,"duration_ms":1,
               "verdict":{{"repo":"acme/tool","tag":"v1.2.3","arch":"x86_64","libc":"gnu",
                 "outcome":"needs_interaction","selected":null,"checksum":null,
                 "candidates":[{},{}]}},
               "trace":[{},{},{},{}]}}"#,
            candidate(cli, "tool-cli"),
            candidate(server, "tool-server"),
            event(
                "asset rejected",
                r#","asset":"tool.exe","reason":"foreign_os","marker":"exe""#
            ),
            ranked(cli, "gnu", 0),
            ranked(server, "gnu", 0),
            event("selection", r#","outcome":"needs_interaction","tied":2"#),
        ));

        assert_eq!(view.outcome, Outcome::NeedsInteraction);
        assert_eq!(view.survivors.len(), 2);
        assert!(view.survivors.iter().all(|s| s.tied && !s.selected));
        assert_eq!(view.stems, vec!["tool-cli", "tool-server"]);
        assert_eq!(view.rejected[0].reason, "foreign_os");
        assert_eq!(view.survivors[0].dims[2].tier, Some(0));
    }

    /// An auto-selected verdict lists only the winner, so the runner-up must still come from
    /// the trace — with its stem honestly unknown rather than guessed.
    #[test]
    fn losers_of_an_auto_selection_come_from_the_trace() {
        let gnu = "rg-15.2.0-x86_64-unknown-linux-gnu.tar.gz";
        let musl = "rg-15.2.0-x86_64-unknown-linux-musl.tar.gz";
        let view = record(&format!(
            r#"{{"repo":"BurntSushi/ripgrep","tag":"15.2.0","arch":"x86_64","libc":"gnu","n_assets":2,
               "outcome":"auto_selected","exit_code":0,"duration_ms":1,
               "verdict":{{"repo":"BurntSushi/ripgrep","tag":"15.2.0","arch":"x86_64","libc":"gnu",
                 "outcome":"auto_selected","selected":{},"checksum":"sha256sums.txt",
                 "candidates":[{}]}},
               "trace":[{},{},{}]}}"#,
            candidate(gnu, "rg"),
            candidate(gnu, "rg"),
            ranked(gnu, "gnu", 0),
            ranked(musl, "musl", 2),
            event("selection", r#","outcome":"auto_selected","tied":1"#),
        ));

        assert_eq!(view.survivors.len(), 2);
        assert!(view.survivors[0].selected);
        assert_eq!(view.survivors[0].stem.as_deref(), Some("rg"));
        assert_eq!(view.survivors[1].stem, None);
        assert_eq!(view.survivors[1].dims[2].tier, Some(2));
        assert!(view.survivors.iter().all(|s| !s.tied));
        assert_eq!(view.checksum.as_deref(), Some("sha256sums.txt"));
    }

    #[test]
    fn search_reaches_asset_names_not_just_the_repo() {
        let view = record(&format!(
            r#"{{"repo":"acme/tool","tag":"v1","arch":"x86_64","libc":"gnu","n_assets":1,
               "outcome":"no_match","exit_code":43,"duration_ms":1,
               "trace":[{}]}}"#,
            event(
                "asset rejected",
                r#","asset":"Tool-x86_64.AppImage.zsync","reason":"not_a_binary","marker":"zsync""#
            ),
        ));

        assert!(view.matches("acme"));
        assert!(view.matches("appimage"));
        assert!(!view.matches("windows"));
    }
}
