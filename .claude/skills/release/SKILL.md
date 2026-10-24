---
name: release
description: Cut a binto release — bump version, regenerate CHANGELOG with git-cliff, commit and tag.
argument-hint: "<version, e.g. 0.3.0>"
disable-model-invocation: true
---

# Release binto $ARGUMENTS

Only run when the user invokes `/release`. Stop and ask if anything below is unexpected.

1. **Preconditions**: working tree clean, on `master`, `$ARGUMENTS` is valid semver and
   greater than the latest `v*` tag (`git tag --sort=-v:refname | head -1`), and
   `git cliff --version` works.
2. **Bump** `version` in `crates/binto/Cargo.toml`, then `cargo check -p binto` so
   `Cargo.lock` picks it up.
3. **CHANGELOG** with git-cliff (configured by `cliff.toml` at the repo root; don't
   hand-edit the generated sections):
   1. Preview the new section: `git cliff --unreleased --tag v$ARGUMENTS --strip all`. Show it to the user.
   2. List commits that weren't recognised as conventional, since they land under "Other":
      `git cliff --unreleased -vv 2>&1 | grep 'did not match'`. If there are any, point them
      out so the user can decide whether that's acceptable. Don't rewrite history to fix them.
   3. Once the user is happy, regenerate the whole file: `git cliff --tag v$ARGUMENTS -o CHANGELOG.md`.
      Regenerate the whole file rather than prepending, so the compare links at the bottom stay
      correct. The version-bump commit made in step 5 is skipped by `cliff.toml`, so generating
      before that commit is correct.
4. **Verify** with the `ci-check` skill. Stop on failure.
5. **Commit** `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` as
   `chore: bump version to v$ARGUMENTS`, then create the annotated tag `v$ARGUMENTS` on
   that commit.
6. **Do not push.** Print the command for the user:
   `git push origin master v$ARGUMENTS` (the tag triggers `.github/workflows/release.yml`).
