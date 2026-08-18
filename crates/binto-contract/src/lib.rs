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

/// Where a candidate landed on each preference dimension.
///
/// Labels rather than ranks: `jq` over a results file should read as the asset name does.
/// Field order is the priority order the matcher compares in — architecture first,
/// packaging last — and two candidates with equal `Tiers` are tied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tiers {
    /// The canonical architecture the asset names, or `unspecified`.
    pub arch: String,
    /// `linux`, or `unspecified` when the asset names no OS.
    pub os: String,
    /// `gnu`, `musl`, or `unspecified`.
    pub libc: String,
    /// `tar`, `zip`, `raw`, or `appimage`.
    pub format: String,
}

/// Something about a candidate the user should know before installing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "note", rename_all = "snake_case")]
pub enum SelectionNote {
    /// A preference existed and the release could not satisfy it — gnu was preferred but
    /// only musl was published. The install still works; it is just not what was asked
    /// for, which the old integer score had no way to express.
    Fallback {
        dimension: String,
        wanted: String,
        got: String,
    },
    /// The asset names nothing on this dimension, so the matcher trusted the publisher
    /// rather than verifying a marker.
    Unspecified { dimension: String },
}

/// One ranked asset in the verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub name: String,
    pub tiers: Tiers,
    /// Empty when every preference was met and every dimension was stated. Only
    /// populated for the selected asset and the leader of a tie.
    #[serde(default)]
    pub notes: Vec<SelectionNote>,
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
    // matcher — one event per decision, in pipeline order.
    /// A hard filter disqualified an asset. Carries `reason` and `marker`.
    pub const ASSET_REJECTED: &str = "asset rejected";
    /// Summary of the hard-filter pass. Carries `before`, `after`, `rejected`.
    pub const APPLIED_HARD_FILTERS: &str = "applied hard filters";
    /// A surviving asset placed on the preference tiers. Carries the four tier labels
    /// and their ranks.
    pub const ASSET_RANKED: &str = "asset ranked";
    /// The outcome. Carries `outcome`, `asset`, `tied`, the tier labels, and `notes`.
    pub const SELECTION: &str = "selection";

    // stored-pattern fast path
    pub const PATTERN_SELECTED: &str = "pattern fast-path selected asset";
    pub const PATTERN_INCONCLUSIVE: &str =
        "pattern fast-path inconclusive, falling back to ranking";
    pub const PATTERN_UNUSABLE: &str =
        "pattern fast-path hit an asset that fails the hard filters, falling back to ranking";
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
            r#"{"timestamp":"t","level":"DEBUG","message":"asset ranked","target":"binto::matcher::rank",
                "asset":"x.tar.gz","libc":"gnu","libc_tier":0,"span":{"name":"match_asset"}}"#,
        )
        .unwrap();
        assert_eq!(ev.message, messages::ASSET_RANKED);
        assert_eq!(ev.field_str("asset"), Some("x.tar.gz"));
        assert_eq!(ev.field_str("libc"), Some("gnu"));
        assert_eq!(ev.field_i64("libc_tier"), Some(0));
        assert!(ev.field("span").is_some());
    }

    #[test]
    fn a_candidate_round_trips_with_its_notes() {
        let json = r#"{
            "name": "rg-x86_64-linux-musl.tar.gz",
            "tiers": {"arch":"x86_64","os":"linux","libc":"musl","format":"tar"},
            "notes": [{"note":"fallback","dimension":"libc","wanted":"gnu","got":"musl"}]
        }"#;
        let c: Candidate = serde_json::from_str(json).unwrap();
        assert_eq!(c.tiers.libc, "musl");
        assert_eq!(
            c.notes,
            vec![SelectionNote::Fallback {
                dimension: "libc".into(),
                wanted: "gnu".into(),
                got: "musl".into(),
            }]
        );
        let back: Candidate = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.notes, c.notes);
    }

    /// A candidate with nothing to report omits `notes` entirely rather than carrying an
    /// empty field the reader has to interpret.
    #[test]
    fn notes_default_to_empty() {
        let c: Candidate = serde_json::from_str(
            r#"{"name":"t","tiers":{"arch":"x86_64","os":"linux","libc":"gnu","format":"tar"}}"#,
        )
        .unwrap();
        assert!(c.notes.is_empty());
    }
}
