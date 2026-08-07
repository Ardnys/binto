//! Matcher test runner for binto.
//!
//! Reads a JSONL dataset of GitHub releases (one repo per line, as collected by
//! gh-releases-data-script), pipes each line into `binto match` on stdin, and records
//! everything the command produces: the stdout verdict, the stderr decision trace, and
//! the exit code.
//!
//! Every shape crossing the binto boundary — the verdict, the outcomes, the exit codes,
//! the environment that turns on the JSON trace — comes from `binto-contract`, so a change
//! to binto's output breaks this build instead of silently skewing results.
//!
//! Output is a results JSONL (one record per repo) plus a human summary on stderr.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use binto_contract::{Asset, MatchVerdict, Outcome, TraceEvent, env as binto_env};
use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
#[command(
    name = "runner",
    about = "Run a release dataset through `binto match` and collect verdicts + decision traces"
)]
struct Cli {
    /// Dataset JSONL: one release object per line (from gh-releases-data-script)
    #[arg(short, long, default_value = "harness/datasets/cli.jsonl")]
    dataset: PathBuf,

    /// Path to the binto binary to test
    #[arg(short, long, default_value = "target/release/binto")]
    binto: PathBuf,

    /// Target architecture passed to `binto match --arch`
    #[arg(long, default_value = "x86_64")]
    arch: String,

    /// Libc preference passed to `binto match --libc` (gnu|musl)
    #[arg(long, default_value = "gnu")]
    libc: String,

    /// Results JSONL path [default: results-<arch>-<libc>.jsonl]
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Stop after running N repos
    #[arg(long, value_name = "N")]
    limit: Option<usize>,

    /// Only run repos whose owner/repo contains this substring
    #[arg(long, value_name = "SUBSTR")]
    repo: Option<String>,

    /// Print one progress line per repo instead of just the summary
    #[arg(short, long)]
    verbose: bool,
}

/// The slice of a dataset line the runner itself needs; the raw line is what gets piped to
/// binto (which ignores the metadata fields).
#[derive(Deserialize)]
struct DatasetEntry {
    repo: String,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// One stderr line: a decision event, or the raw text if it wasn't binto's JSON log
/// (a panic, for instance) so nothing is silently dropped.
#[derive(Serialize)]
#[serde(untagged)]
enum TraceLine {
    Event(TraceEvent),
    Raw { raw: String },
}

/// What the runner concluded about a single invocation.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum RunOutcome {
    /// binto produced a verdict whose outcome agrees with its exit code.
    Matched(Outcome),
    /// binto crashed, printed no parseable verdict, or contradicted its own exit code.
    Error,
}

impl RunOutcome {
    /// Key used for the summary table and for `jq` filtering of the results file.
    fn label(&self) -> String {
        match self {
            RunOutcome::Matched(o) => o.to_string(),
            RunOutcome::Error => "error".to_string(),
        }
    }
}

/// One line of the results file: everything `binto match` returned for one repo.
#[derive(Serialize)]
struct RunRecord<'a> {
    repo: &'a str,
    tag: Option<&'a str>,
    arch: &'a str,
    libc: &'a str,
    n_assets: usize,
    /// `auto_selected` / `needs_interaction` / `no_match`, or `error`.
    outcome: String,
    exit_code: Option<i32>,
    duration_ms: u128,
    /// binto's stdout verdict (absent when it could not be parsed).
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict: Option<MatchVerdict>,
    /// Every stderr decision event.
    trace: Vec<TraceLine>,
    /// Diagnostic detail, only when `outcome` is `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

struct Output {
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Spawn `binto match`, feed the dataset line on stdin, capture everything.
fn run_match(
    cli: &Cli,
    repo: &str,
    raw_line: &str,
    scratch_config: &str,
) -> Result<(Output, u128)> {
    let start = Instant::now();
    let mut child = Command::new(&cli.binto)
        .args([
            "-vv", "match", repo, "--arch", &cli.arch, "--libc", &cli.libc,
        ])
        // Machine-readable stderr trace.
        .env(binto_env::LOG_FORMAT, binto_env::LOG_FORMAT_JSON)
        // No file-log spam from hundreds of invocations.
        .env(binto_env::LOG, binto_env::LOG_OFF)
        // Don't read the user's real binto config.
        .env("XDG_CONFIG_HOME", scratch_config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {}", cli.binto.display()))?;

    // Write the release JSON and drop the handle so binto sees EOF on stdin.
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(raw_line.as_bytes())
        .context("failed to write release JSON to binto stdin")?;

    // wait_with_output drains stdout and stderr concurrently — no pipe deadlock.
    let out = child.wait_with_output().context("binto did not exit")?;
    let duration_ms = start.elapsed().as_millis();

    Ok((
        Output {
            status_code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        duration_ms,
    ))
}

/// Parse stderr JSONL into decision events, keeping anything unparseable verbatim.
fn parse_trace(stderr: &str) -> Vec<TraceLine> {
    stderr
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match serde_json::from_str::<TraceEvent>(l) {
            Ok(ev) => TraceLine::Event(ev),
            Err(_) => TraceLine::Raw { raw: l.to_string() },
        })
        .collect()
}

/// Decide what happened, cross-checking the verdict against the exit code. Any
/// disagreement is an `Error`: the two are supposed to encode the same thing.
fn classify(verdict: &Option<MatchVerdict>, output: &Output) -> (RunOutcome, Option<String>) {
    match verdict {
        Some(v) => {
            let expected = v.outcome.exit_code();
            if output.status_code == Some(expected) {
                (RunOutcome::Matched(v.outcome), None)
            } else {
                (
                    RunOutcome::Error,
                    Some(format!(
                        "verdict says '{}' (exit {expected}) but binto exited with {:?}",
                        v.outcome, output.status_code
                    )),
                )
            }
        }
        None => (
            RunOutcome::Error,
            Some(format!(
                "no parseable verdict on stdout (exit {:?}); stderr tail: {}",
                output.status_code,
                output.stderr.lines().last().unwrap_or("")
            )),
        ),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if !matches!(cli.libc.as_str(), "gnu" | "musl") {
        bail!("--libc must be 'gnu' or 'musl', got '{}'", cli.libc);
    }
    if !cli.binto.exists() {
        bail!(
            "binto binary not found at {} — build it first with `cargo build --release -p binto`, \
             or pass --binto",
            cli.binto.display()
        );
    }

    let out_path = cli
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("results-{}-{}.jsonl", cli.arch, cli.libc)));

    // Empty config dir => binto uses its built-in defaults; --arch/--libc pin the rest.
    let scratch_config = std::env::temp_dir().join("binto-runner-config");
    std::fs::create_dir_all(&scratch_config).context("failed to create scratch config dir")?;
    let scratch_config = scratch_config.to_string_lossy().into_owned();

    let dataset = File::open(&cli.dataset)
        .with_context(|| format!("failed to open dataset {}", cli.dataset.display()))?;
    let mut out = BufWriter::new(
        File::create(&out_path)
            .with_context(|| format!("failed to create results file {}", out_path.display()))?,
    );

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors: Vec<String> = Vec::new();
    let mut ran = 0usize;
    let run_start = Instant::now();

    for (lineno, line) in BufReader::new(dataset).lines().enumerate() {
        let line = line.context("failed to read dataset line")?;
        if line.trim().is_empty() {
            continue;
        }

        let entry: DatasetEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(e) => {
                eprintln!(
                    "warning: skipping malformed dataset line {}: {e}",
                    lineno + 1
                );
                continue;
            }
        };
        if let Some(filter) = &cli.repo
            && !entry.repo.contains(filter.as_str())
        {
            continue;
        }
        if let Some(limit) = cli.limit
            && ran >= limit
        {
            break;
        }
        ran += 1;

        let (output, duration_ms) = run_match(&cli, &entry.repo, &line, &scratch_config)?;
        let verdict: Option<MatchVerdict> = serde_json::from_str(&output.stdout).ok();
        let (run_outcome, error) = classify(&verdict, &output);
        let label = run_outcome.label();

        let record = RunRecord {
            repo: &entry.repo,
            tag: entry.tag.as_deref(),
            arch: &cli.arch,
            libc: &cli.libc,
            n_assets: entry.assets.len(),
            outcome: label.clone(),
            exit_code: output.status_code,
            duration_ms,
            verdict,
            trace: parse_trace(&output.stderr),
            error: error.clone(),
        };
        serde_json::to_writer(&mut out, &record).context("failed to serialize result")?;
        out.write_all(b"\n")?;

        *counts.entry(label.clone()).or_default() += 1;
        if let Some(err) = error {
            errors.push(format!("{}: {err}", entry.repo));
        }
        if cli.verbose {
            eprintln!("[{ran}] {} -> {label} ({duration_ms}ms)", entry.repo);
        } else if ran.is_multiple_of(25) {
            eprintln!("... {ran} repos done");
        }
    }

    out.flush().context("failed to flush results file")?;

    // -- summary ----------------------------------------------------------
    let total: usize = counts.values().sum();
    eprintln!(
        "\n=== {total} repos in {:.1}s -> {} (target {}/{})",
        run_start.elapsed().as_secs_f32(),
        out_path.display(),
        cli.arch,
        cli.libc,
    );
    for (outcome, n) in &counts {
        eprintln!(
            "  {outcome:<18} {n:>4}  ({:.1}%)",
            *n as f64 * 100.0 / total.max(1) as f64
        );
    }
    if !errors.is_empty() {
        eprintln!("\nerrors:");
        for e in &errors {
            eprintln!("  {e}");
        }
    }

    // Non-zero exit if anything hard-failed, so CI can gate on it.
    if counts.contains_key("error") {
        std::process::exit(1);
    }
    Ok(())
}
