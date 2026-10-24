---
name: release
description: Cut a binto release — bump version, update CHANGELOG, commit and tag.
argument-hint: "<version, e.g. 0.3.0>"
disable-model-invocation: true
---

# Release binto $ARGUMENTS

Only run when the user invokes `/release`. Stop and ask if anything below is unexpected.

1. **Preconditions**: working tree clean, on `master`, `$ARGUMENTS` is valid semver and
   greater than the latest `v*` tag (`git tag --sort=-v:refname | head -1`).
2. **Bump** `version` in `crates/binto/Cargo.toml`, then `cargo check -p binto` so
   `Cargo.lock` picks it up.
3. **CHANGELOG**: collect `git log <last-tag>..HEAD --pretty=%s` and prepend a
   `## [$ARGUMENTS] - <today YYYY-MM-DD>` section to `CHANGELOG.md` using the existing
   headings, mapping conventional-commit types:
   `feat`→`### 🚀 Features`, `fix`→`### 🐛 Bug Fixes`, `refactor`→`### 🚜 Refactor`,
   `perf`→`### ⚡ Performance`, `chore`/`ci`/`build`→`### ⚙️ Miscellaneous Tasks`.
   Breaking changes (`!`) get a `[**breaking**] ` prefix. Capitalise the first word,
   drop the type prefix. Show the section to the user before continuing.
4. **Verify** with the `ci-check` skill. Stop on failure.
5. **Commit** `chore: bump version to v$ARGUMENTS` and create annotated tag `v$ARGUMENTS`.
6. **Do not push.** Print the command for the user:
   `git push origin master v$ARGUMENTS` (the tag triggers `.github/workflows/release.yml`).
