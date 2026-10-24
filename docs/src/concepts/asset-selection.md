# How binto picks an asset

A release usually ships many assets: builds for other systems, checksums, source archives. binto
picks the one to install in two passes.

## 1. Drop what can't run here

These assets are never candidates:

- builds for another operating system (Windows, macOS, …) or another architecture
- checksum and signature files, and SBOMs
- source archives
- distribution packages such as `.deb` and `.rpm` (binto installs into your home directory,
  not into a system package database)
- archive formats binto can't open

Architectures are recognised by their common spellings: `amd64` and `x64` count as `x86_64`,
`arm64` as `aarch64`, and so on.

## 2. Rank what's left

The remaining assets can all be installed, so the question is which is best. binto compares
them on four things, in this order, and a later one only matters when the earlier ones are
equal:

1. **Architecture:** an asset that names your architecture beats one that doesn't say.
2. **Operating system:** an asset that says `linux` beats one that doesn't say.
3. **libc:** the one matching [`prefer_libc`](../reference/configuration.md) (`gnu` by
   default) beats the other. A release that only ships the other one still installs.
4. **Format:** `.tar.gz`/`.tgz`, then `.tar.xz`, `.tar.bz2`, `.zip`, a bare binary, and an
   AppImage last. Tarballs come first because they keep the executable bit and often carry
   man pages and shell completions.

A worse rank on one point is never made up for by a better rank on a later one: a
`.tar.gz` for the wrong libc doesn't beat a `.zip` for the right one.

## When it can't decide

If two assets end up equal on all four, binto doesn't guess. It shows you the candidates and
asks. With `-y` it picks one and tells you which.

binto also tells you when its pick isn't quite what you asked for, for example a musl build
because the release has no glibc one.

## Updates

binto remembers the asset it installed. When you update, it looks for the same kind of asset
in the new release, so you aren't asked again just because the version in the file name
changed.
