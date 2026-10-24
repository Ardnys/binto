---
name: debug-match
description: Explain why the asset matcher picked (or failed to pick) an asset for one GitHub repo, and point at the code responsible. Use when the user asks "why did X not auto-select", "why did it pick musl for X", or reports a wrong asset for a repo.
argument-hint: "<owner/repo> [arch] [libc]"
---

# Debug one repo's match

Target: `$ARGUMENTS` (first word = repo, optional arch, default `x86_64`; optional libc, default `gnu`).

## Steps

1. Make sure the repo is in `harness/datasets/cli.jsonl` (`grep -c '"repo": "<repo>"'`).
   If it isn't, say so and stop — fetching new release data is out of scope here.
2. Run just that repo into a scratch file:
   ```sh
   cargo run -p runner -- --repo <repo> --arch <arch> --libc <libc> -o $SCRATCH/one.jsonl -v
   ```
3. Extract the decision trail:
   ```sh
   jq '{outcome, selected: .verdict.selected.name, notes: .verdict.selected.notes}' $SCRATCH/one.jsonl
   jq -c '.verdict.candidates[] | {name, tiers}' $SCRATCH/one.jsonl
   jq -r '.trace[] | select(.message=="asset rejected") | "\(.reason)\t\(.marker)\t\(.asset)"' $SCRATCH/one.jsonl
   ```
4. Map the decision to code:
   - rejected by a hard filter → `crates/binto/src/matcher/filter.rs`
   - wrong arch/os/libc/format fact extracted from a name → `crates/binto/src/matcher/facts.rs`
   - tie or ordering → `crates/binto/src/matcher/rank.rs` (`tie_group_len`, `PreferenceProfile`)
   - stem/variant naming → `crates/binto/src/matcher/mod.rs`

## Report

1. The outcome in one sentence and **why** (which dimension decided it, or which tie).
2. Whether this looks like a matcher bug or a genuinely ambiguous release.
3. If a bug: the function to change, and a proposed unit test for that module's
   `#[cfg(test)] mod tests` block that reproduces it with the real asset names.
   Don't edit code until the user agrees.
