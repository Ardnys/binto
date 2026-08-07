# runner

Matcher test runner for binto. Feeds a JSONL dataset of GitHub releases (collected
by `gh-releases-data-script`) through `binto match`, one repo per invocation, and
records everything binto returns: the **stdout verdict**, the **stderr decision
trace**, and the **exit code**.

All the shapes crossing the binto boundary come from the
[`binto-contract`](../../crates/binto-contract) crate, so a change to binto's
output breaks this build instead of silently skewing results.

## How it works

For each dataset line, the runner spawns:

```
BINTO_LOG_FORMAT=json BINTO_LOG=off binto -vv match <repo> --arch <arch> --libc <libc>
```

and pipes the raw dataset line to stdin (`binto match` reads stdin by default and
ignores the dataset's metadata fields). `XDG_CONFIG_HOME` points at a scratch dir
so runs never read your real binto config; `BINTO_LOG=off` disables binto's file
log so hundreds of invocations don't spam it.

The exit code encodes the outcome (`0` auto-selected, `42` needs interaction,
`43` no compatible asset — see `binto_contract::exit_code`). The runner
cross-checks it against the verdict's `outcome` and flags any disagreement as an
`error`.

## Usage

Run from the **workspace root**; the defaults assume that working directory.

```sh
cargo build --release              # builds binto and the runner
cargo run --release -p runner      # whole dataset, x86_64/gnu

# other targets — each run writes its own results file
cargo run --release -p runner -- --arch aarch64 --libc musl

# quick dry run / single-repo debugging
cargo run -p runner -- --limit 5 -v
cargo run -p runner -- --repo ripgrep -v
```

| Flag | Meaning |
| --- | --- |
| `-d`/`--dataset` | Dataset JSONL (default `harness/datasets/cli.jsonl`). |
| `-b`/`--binto` | binto binary to test (default `target/release/binto`). |
| `--arch`, `--libc` | Match target (default `x86_64` / `gnu`). |
| `-o`/`--output` | Results path (default `results-<arch>-<libc>.jsonl`). |
| `--limit N` | Only run the first N repos. |
| `--repo SUBSTR` | Only run repos whose `owner/repo` contains SUBSTR. |
| `-v` | One progress line per repo. |

The runner exits non-zero only if some repo produced an `error` (binto crash,
unparseable verdict, or verdict/exit-code mismatch) — `needs_interaction` and
`no_match` are normal data points, not failures.

## Results format

One JSON object per repo:

```json
{
  "repo": "BurntSushi/ripgrep",
  "tag": "15.1.0",
  "arch": "x86_64",
  "libc": "gnu",
  "n_assets": 28,
  "outcome": "auto_selected",
  "exit_code": 0,
  "duration_ms": 5,
  "verdict": { "outcome": "auto_selected", "selected": {...}, "checksum": "...", "candidates": [...] },
  "trace": [ { "message": "scored asset", "asset": "...", "total": 1900, ... }, ... ],
  "error": "only present when outcome is \"error\""
}
```

- `verdict` is binto's stdout verdict (`binto_contract::MatchVerdict`).
- `trace` is every stderr decision event (`binto_contract::TraceEvent`): scoring
  components, filter rejections, the confidence-gap decision, checksum discovery.
  A line that isn't binto's JSON log is kept as `{"raw": "..."}` so nothing is lost.
  `binto_contract::messages` has a constant per decision point.

### Digging into a result

```sh
# why did repo X not auto-select?
jq -c 'select(.repo=="hatoo/oha") | .trace[] | {message, asset, total, gap}' results-x86_64-gnu.jsonl

# outcome counts
jq -r .outcome results-x86_64-gnu.jsonl | sort | uniq -c

# auto-selected repos where no checksum file was found
jq -r 'select(.outcome=="auto_selected" and .verdict.checksum==null) | .repo' results-x86_64-gnu.jsonl
```

## Baseline (cli.jsonl, 456 repos, x86_64/gnu, binto 0.2.0)

| outcome | count | share |
| --- | --- | --- |
| auto_selected | 395 | 86.6% |
| needs_interaction | 51 | 11.2% |
| no_match | 10 | 2.2% |
| error | 0 | — |

Also notable: 244 of the 395 auto-selected repos had **no checksum file
discovered** — checksum discovery is the biggest remaining improvement surface.
