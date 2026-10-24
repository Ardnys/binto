---
name: matcher-regression
description: Sweep the asset matcher over the whole release dataset and compare against the baseline. Use after any change under crates/binto/src/matcher/ or crates/binto-contract/, or when asked whether a matcher change made things better or worse.
argument-hint: "[previous-results-dir]"
---

# Matcher regression sweep

The matcher is validated by the harness in `harness/runner` (see its README). Run from the
workspace root.

## Steps

1. Build: `cargo build --release` (builds `binto` and `runner`).
2. Pick an output dir: a fresh `results-<short-git-sha>/` in the scratchpad dir (never
   overwrite the user's `results-*.jsonl` in the repo root).
3. Run the three baseline targets:
   ```sh
   cargo run --release -p runner -- --arch x86_64  --libc gnu  -o $OUT/results-x86_64-gnu.jsonl
   cargo run --release -p runner -- --arch x86_64  --libc musl -o $OUT/results-x86_64-musl.jsonl
   cargo run --release -p runner -- --arch aarch64 --libc gnu  -o $OUT/results-aarch64-gnu.jsonl
   ```
   A non-zero exit means some repo hit `error` — that is always a regression; report it first.
4. Outcome counts per file: `jq -r .outcome FILE | sort | uniq -c`.
   Compare with the **Baseline** table in `harness/runner/README.md`.
5. If a previous results dir was given (`$ARGUMENTS`), or `results-*.jsonl` files exist in the
   repo root, diff per repo with the `cmp()` join from [reference.md](reference.md).

## Report

A short table: target × outcome, with the baseline and the delta. Then list:
- repos that moved **to** `no_match` / `needs_interaction` / `error` (regressions),
- repos that moved to `auto_selected` (wins),
- `PICK` changes (same outcome, different asset) — these need a human eye.

For deeper pattern analysis of the results, suggest the `matcher-analyst` agent instead of
reading the JSONL into this context. Do **not** update the README baseline unless asked.
