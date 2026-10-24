---
name: ci-check
description: Run binto's CI gate locally (fmt, clippy -D warnings, workspace tests). Use before committing, before claiming a change is done, or when the user asks "does CI pass?".
allowed-tools: Bash(cargo fmt*), Bash(cargo clippy*), Bash(cargo test*)
---

# CI check

Mirror `.github/workflows/ci.yml` exactly, in this order, and stop at the first failure:

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`
3. `cargo test --workspace --locked`

## Reporting

- All green: one line, e.g. `CI check passed (fmt, clippy, N tests)`.
- On failure: name the step that failed and quote **only** the relevant error lines
  (file:line + message), not the whole cargo output.
- If `fmt` fails, offer to run `cargo fmt` — do not run it unasked.
- Never "fix" a clippy lint with `#[allow(...)]` unless the user agrees.
