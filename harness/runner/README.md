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
  "trace": [ { "message": "asset ranked", "asset": "...", "libc": "gnu", ... }, ... ],
  "error": "only present when outcome is \"error\""
}
```

- `verdict` is binto's stdout verdict (`binto_contract::MatchVerdict`).
- `trace` is every stderr decision event (`binto_contract::TraceEvent`): filter
  rejections, tier placements, the selection, checksum discovery. A line that isn't
  binto's JSON log is kept as `{"raw": "..."}` so nothing is lost.
  `binto_contract::messages` has a constant per decision point.

### Candidates, tiers, and notes

The matcher has no score. Each candidate carries the **tier** it landed on per dimension,
and candidates are ordered best-first; two candidates with equal `tiers` are **tied**, and
a tie is what produces `needs_interaction`.

```json
{
  "name": "ripgrep-15.1.0-x86_64-unknown-linux-musl.tar.gz",
  "tiers": { "arch": "x86_64", "os": "linux", "libc": "musl", "format": "tar" },
  "notes": [ { "note": "fallback", "dimension": "libc", "wanted": "gnu", "got": "musl" } ]
}
```

`notes` is the signal a score could never carry: `fallback` means a preference existed and
the release could not satisfy it, `unspecified` means the asset states nothing on that
dimension. Only the selected asset (and the leader of a tie) carries notes.

The four trace messages, in pipeline order: `asset rejected` (with `reason` and `marker`),
`applied hard filters`, `asset ranked`, `selection` (with `outcome`, `tied`, and `notes`).

### Digging into a result

```sh
# why did repo X not auto-select? — the tie group is the leading run of equal tiers
jq -c 'select(.repo=="hatoo/oha") | .verdict.candidates[] | {name, tiers}' results-x86_64-gnu.jsonl

# what got thrown out, and why
jq -r 'select(.repo=="hatoo/oha") | .trace[] | select(.message=="asset rejected")
       | "\(.reason)\t\(.marker)\t\(.asset)"' results-x86_64-gnu.jsonl

# outcome counts
jq -r .outcome results-x86_64-gnu.jsonl | sort | uniq -c

# repos installing a libc they did not ask for
jq -r 'select(.verdict.selected.notes[]?.note=="fallback") | .repo' results-x86_64-gnu.jsonl

# repos where the winner states nothing at all — binto is trusting the publisher
jq -r 'select([.verdict.selected.notes[]?.dimension] | contains(["arch"])) | .repo' \
   results-x86_64-gnu.jsonl

# what the hard filters removed across the whole dataset
jq -r '.trace[]? | select(.message=="asset rejected") | .reason' results-x86_64-gnu.jsonl \
  | sort | uniq -c | sort -rn

# auto-selected repos where no checksum file was found
jq -r 'select(.outcome=="auto_selected" and .verdict.checksum==null) | .repo' results-x86_64-gnu.jsonl
```

Comparing two runs needs a join, not `jq -s` — slurping two JSONL files yields one flat
array of records, not one array per file:

```sh
cmp() {   # cmp before.jsonl after.jsonl
  paste <(jq -r '"\(.repo)|\(.outcome)|\(.verdict.selected.name // "-")"' "$1" | sort) \
        <(jq -r '"\(.repo)|\(.outcome)|\(.verdict.selected.name // "-")"' "$2" | sort) \
  | awk -F'\t' '{split($1,a,"|"); split($2,b,"|");
      if (a[2]!=b[2])                  printf "%-28s %s -> %s\n", a[1], a[2], b[2];
      else if (a[3]!=b[3])             printf "%-28s PICK %s -> %s\n", a[1], a[3], b[3] }'
}
```

## Baseline (cli.jsonl, 456 repos, binto 0.2.0, tier matcher)

| outcome | x86_64/gnu | x86_64/musl | aarch64/gnu |
| --- | --- | --- | --- |
| auto_selected | 420 (92.1%) | 421 (92.3%) | 370 (81.1%) |
| needs_interaction | 23 (5.0%) | 22 (4.8%) | 15 (3.3%) |
| no_match | 13 (2.9%) | 13 (2.9%) | 71 (15.6%) |
| error | 0 | 0 | 0 |

The previous score-based matcher scored 395/51/10 on x86_64/gnu. The `no_match` count rose
because assets that could never have installed — bare `.gz`/`.bz2`/`.tar.zst` the extractor
cannot open, and `.json`/`.txt` files that used to rank as extensionless binaries — are now
rejected instead of confidently selected. Three repos previously auto-selected
`dist-manifest.json`, `latest.json`, and `sha1sum.txt` **as the binary**.

Remaining improvement surfaces, x86_64/gnu:

- **194 of 420** auto-selected repos had no checksum file discovered.
- **34** auto-select a musl build because no gnu build is published (`notes[].note ==
  "fallback"`) — visible now rather than indistinguishable from a real match.
- **28** auto-select an asset that names no architecture at all.
