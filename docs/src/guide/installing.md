# Installing tools

```sh
binto install BurntSushi/ripgrep
binto i https://github.com/cli/cli      # `i` is short for `install`
binto i sharkdp/fd --prerelease         # include pre-releases in the list
binto i sharkdp/bat -t v0.24.0          # install (and pin) an exact release
binto i junegunn/fzf --to ~/bin         # install into another directory
binto i sharkdp/fd -a fdfind            # install under another name
binto i sharkdp/fd -y                   # no questions: latest release, best asset
```

The repository can be written as `owner/repo` or as any `github.com` URL, with or without the
scheme. Anything after the repository in the URL is ignored.

## What happens

1. binto lists the repository's releases for you to pick from. With `-t` it takes that tag;
   with `-y` it takes the latest release.
2. It picks the asset built for your machine. When the release leaves a real choice, it asks.
   See [How binto picks an asset](../concepts/asset-selection.md).
3. It downloads the asset and verifies it against the release's checksum file, when the
   release publishes one.
4. It extracts the binary, installs it into the install directory, and records the tool in
   its state and in the [manifest](manifest.md).

If a binary of the same name that binto doesn't manage is already on your `PATH`, binto asks
before installing over it.

## What the tool is called

A tool is installed, and tracked, under the name of the binary itself:

1. the name of the executable inside the archive, so `BurntSushi/ripgrep` installs `rg` and
   `cli/cli` installs `gh`;
2. for an asset that is the binary itself, the asset's name without its version, platform
   and extension, so `jq-linux-amd64` installs `jq`;
3. only when neither says anything, the repository's name.

`--alias` overrides all of these. If another tool binto manages already has the name, the
install stops and asks you to pick an alias.

## Options

**`-t, --tag <TAG>`** installs that exact release and pins the tool to it. A pinned tool is
left alone by `binto update`; see [Updating and pinning](updating.md). Running
`binto install <repo> -t <other-tag>` on a tool binto already manages moves it to that tag in
place.

**`-a, --alias <NAME>`** installs the binary under `NAME` instead of its own name.
`binto install sharkdp/fd -a fdfind` installs `fdfind`. The alias is the name binto tracks the
tool by (`binto update fdfind`, `binto remove fdfind`), and it's recorded in the manifest so
`binto sync` reproduces it. The binary inside the archive is still found as usual; only the
installed file is renamed.

**`--to <PATH>`** installs into `PATH` instead of the configured
[`install_dir`](../reference/configuration.md). A leading `~` is expanded. Later updates
reinstall the tool in the same place. This choice belongs to this machine and isn't written to
the manifest.

**`--prerelease`** includes pre-releases in the release list. Set
[`include_prereleases`](../reference/configuration.md) to make that the default.

**`-y, --yes`** never asks: binto takes the latest release (unless `-t` is given), the best
asset, and installs over an unmanaged binary of the same name.

## Repositories with several binaries

Some releases ship more than one tool, like `ozwaldorf/lutgen-rs` with `lutgen-cli` and
`lutgen-studio`. Installing such a repository asks which one you want. Run the same
`binto install` again to add another: binto offers only the binaries you don't have yet. Each
one is tracked, pinned and updated on its own.
