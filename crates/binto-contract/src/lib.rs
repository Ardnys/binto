//! The contract between `binto match` and the matcher test harness.
//!
//! Everything that crosses that boundary lives here so both sides are compiled against
//! one definition and cannot drift: the release read on stdin ([`MatchInput`]), the
//! verdict written to stdout ([`MatchVerdict`]), the exit codes that encode the outcome
//! ([`Outcome::exit_code`]), the environment that puts binto in machine-readable mode
//! ([`env`]), and the shape of the decision trace on stderr ([`TraceEvent`]).
//!
//! This crate is deliberately dependency-light (serde only) so the harness, and later the
//! container runner and collector, can depend on it without pulling in the CLI.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// -- input ---------------------------------------------------------------

/// A release asset. Only `name` is meaningful to the matcher; the rest is carried so a
/// dataset line round-trips and the installer has what it needs to download.
///
/// The extra fields default because harness fixtures are commonly hand-written as
/// `{"name": "..."}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    #[serde(default)]
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub content_type: String,
}

/// A release fed to `binto match` on stdin or via `--file`.
///
/// Deserializes both a raw GitHub release response (`tag_name`, `assets`) and a
/// hand-written fixture (`{"tag": "...", "assets": [...]}`); unknown fields — such as the
/// dataset's `stars`/`topics`/`language` metadata — are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchInput {
    #[serde(alias = "tag_name")]
    pub tag: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

// -- output --------------------------------------------------------------

/// What the matcher concluded. Serializes to the snake_case strings the verdict carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// One asset won outright — either the only candidate, or ahead by the confidence gap.
    AutoSelected,
    /// Several assets scored too close together to pick without asking the user.
    NeedsInteraction,
    /// Nothing compatible survived filtering and scoring.
    NoMatch,
}

impl Outcome {
    /// The process exit code binto uses to encode this outcome.
    pub const fn exit_code(self) -> i32 {
        match self {
            Outcome::AutoSelected => exit_code::AUTO_SELECTED,
            Outcome::NeedsInteraction => exit_code::NEEDS_INTERACTION,
            Outcome::NoMatch => exit_code::NO_MATCH,
        }
    }

    /// The outcome an exit code stands for, or `None` if binto failed for another reason.
    pub const fn from_exit_code(code: i32) -> Option<Self> {
        match code {
            exit_code::AUTO_SELECTED => Some(Outcome::AutoSelected),
            exit_code::NEEDS_INTERACTION => Some(Outcome::NeedsInteraction),
            exit_code::NO_MATCH => Some(Outcome::NoMatch),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Outcome::AutoSelected => "auto_selected",
            Outcome::NeedsInteraction => "needs_interaction",
            Outcome::NoMatch => "no_match",
        }
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Exit codes `binto match` uses so a harness can read the outcome without parsing stdout.
pub mod exit_code {
    pub const AUTO_SELECTED: i32 = 0;
    pub const NEEDS_INTERACTION: i32 = 42;
    pub const NO_MATCH: i32 = 43;
}

/// One ranked asset in the verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub name: String,
    pub score: i32,
    /// How the asset's architecture matched: `EXACT`, `SYNONYM`, or `NONE`.
    pub arch_match: String,
}

/// The machine-readable result `binto match` writes to stdout — one JSON object, always
/// emitted regardless of log verbosity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchVerdict {
    pub repo: String,
    pub tag: String,
    pub arch: String,
    pub libc: String,
    pub outcome: Outcome,
    /// The chosen asset, present only for [`Outcome::AutoSelected`].
    pub selected: Option<Candidate>,
    /// The checksum asset covering `selected`, if one was found.
    pub checksum: Option<String>,
    /// Every positively-scored asset, ranked best-first.
    pub candidates: Vec<Candidate>,
}

// -- trace ---------------------------------------------------------------

/// Environment that puts binto into machine-readable mode.
pub mod env {
    /// Selects the terminal log format. Set to [`LOG_FORMAT_JSON`] for a parseable trace.
    pub const LOG_FORMAT: &str = "BINTO_LOG_FORMAT";
    pub const LOG_FORMAT_JSON: &str = "json";
    /// Controls the rotating file log; set to [`LOG_OFF`] to disable it entirely.
    pub const LOG: &str = "BINTO_LOG";
    pub const LOG_OFF: &str = "off";
}

/// One line of the decision trace binto writes to stderr under
/// `BINTO_LOG_FORMAT=json` with `-v`/`-vv`.
///
/// The envelope is typed; every event-specific field (`asset`, `total`, `reason`, `gap`,
/// the `span`/`spans` objects, …) lands in [`fields`](Self::fields) so nothing is lost and
/// new instrumentation needs no change here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub timestamp: String,
    pub level: String,
    /// The event's static message — see [`messages`] for the decision points.
    #[serde(default)]
    pub message: String,
    /// Emitting module, e.g. `binto::matcher::filter`.
    #[serde(default)]
    pub target: String,
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

impl TraceEvent {
    /// Borrow an event-specific field by name.
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.fields.get(name)
    }

    pub fn field_str(&self, name: &str) -> Option<&str> {
        self.fields.get(name).and_then(Value::as_str)
    }

    pub fn field_i64(&self, name: &str) -> Option<i64> {
        self.fields.get(name).and_then(Value::as_i64)
    }
}

/// The `message` of each decision point, so analysis code matches on a constant instead of
/// a string literal copied out of the matcher.
pub mod messages {
    // matcher
    pub const FILTERED_OUT_ASSET: &str = "filtered out asset";
    pub const APPLIED_HARD_FILTERS: &str = "applied hard filters";
    pub const SCORED_ASSET: &str = "scored asset";
    pub const ASSET_EXCLUDED: &str = "asset excluded from ranking";
    pub const RANKED_ASSET: &str = "ranked asset";
    pub const AUTO_SELECTED_ASSET: &str = "auto-selected asset";
    pub const NEEDS_INTERACTION: &str = "confidence gap below threshold, needs interaction";
    pub const NO_CANDIDATES_AFTER_FILTERS: &str = "no candidates left after hard filters";
    pub const NO_CANDIDATES_AFTER_SCORING: &str = "no candidates left after scoring";

    // stored-pattern fast path
    pub const PATTERN_SELECTED: &str = "pattern fast-path selected asset";
    pub const PATTERN_INCONCLUSIVE: &str =
        "pattern fast-path inconclusive, falling back to scoring";
    pub const PATTERN_INVALID_GLOB: &str = "stored pattern is not a valid glob";
    pub const PATTERN_MATCHED: &str = "matched stored pattern";

    // checksum discovery / verification
    pub const CHECKSUM_ASSET_FOUND: &str = "found checksum asset";
    pub const CHECKSUM_ASSET_NOT_FOUND: &str = "no checksum asset found";
    pub const CHECKSUM_ENTRY_FOUND: &str = "found checksum entry";
    pub const CHECKSUM_ENTRY_NOT_FOUND: &str = "no checksum entry for file";
    pub const CHECKSUM_COMPARING: &str = "comparing checksums";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_serializes_to_the_documented_strings() {
        for (outcome, text) in [
            (Outcome::AutoSelected, "auto_selected"),
            (Outcome::NeedsInteraction, "needs_interaction"),
            (Outcome::NoMatch, "no_match"),
        ] {
            assert_eq!(
                serde_json::to_string(&outcome).unwrap(),
                format!("\"{text}\"")
            );
            assert_eq!(outcome.as_str(), text);
            assert_eq!(
                serde_json::from_str::<Outcome>(&format!("\"{text}\"")).unwrap(),
                outcome
            );
        }
    }

    #[test]
    fn exit_codes_round_trip() {
        for outcome in [
            Outcome::AutoSelected,
            Outcome::NeedsInteraction,
            Outcome::NoMatch,
        ] {
            assert_eq!(Outcome::from_exit_code(outcome.exit_code()), Some(outcome));
        }
        assert_eq!(Outcome::from_exit_code(1), None);
    }

    #[test]
    fn match_input_reads_a_github_release_and_a_minimal_fixture() {
        let gh: MatchInput = serde_json::from_str(
            r#"{"tag_name":"v1.2.3","stars":42,"assets":[
                 {"name":"t.tar.gz","browser_download_url":"u","size":9,"content_type":"application/gzip"}]}"#,
        )
        .unwrap();
        assert_eq!(gh.tag.as_deref(), Some("v1.2.3"));
        assert_eq!(gh.assets[0].size, 9);

        let minimal: MatchInput =
            serde_json::from_str(r#"{"assets":[{"name":"t.tar.gz"}]}"#).unwrap();
        assert_eq!(minimal.tag, None);
        assert_eq!(minimal.assets[0].name, "t.tar.gz");
        assert_eq!(minimal.assets[0].size, 0);
    }

    #[test]
    fn trace_event_keeps_unmodelled_fields() {
        let ev: TraceEvent = serde_json::from_str(
            r#"{"timestamp":"t","level":"DEBUG","message":"scored asset","target":"binto::matcher::score",
                "asset":"x.tar.gz","total":1900,"span":{"name":"match_asset"}}"#,
        )
        .unwrap();
        assert_eq!(ev.message, messages::SCORED_ASSET);
        assert_eq!(ev.field_str("asset"), Some("x.tar.gz"));
        assert_eq!(ev.field_i64("total"), Some(1900));
        assert!(ev.field("span").is_some());
    }
}
