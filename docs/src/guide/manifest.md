# The manifest and sync

`~/.config/binto/manifest.toml` is a portable list of the tools binto manages. Keep it in your
dotfiles and replay it on another machine with `binto sync` to get the same setup.

`binto install`, `binto adopt` and `binto remove` keep it up to date. You can also edit it by
hand: binto rewrites only the entry it's changing, so your comments, ordering, blank lines and
commented-out entries survive.

## Format

Each repository is a key under `[tools]`. Its value is either a tag, or a table with `tag`
and/or `alias`. The tag `"latest"` tracks the newest release; any other tag pins the tool to
that release.

```toml
[tools]
"sharkdp/bat" = "latest"
"cli/cli" = "v2.45.0"                     # pinned to this release
"sharkdp/fd" = { alias = "fdfind" }       # installed and tracked as `fdfind`

# comment an entry out to keep it around without syncing it
# "junegunn/fzf" = "latest"

# a repository shipping several binaries lists them under `binaries`
[tools."ozwaldorf/lutgen-rs".binaries]
lutgen-cli = "latest"
lutgen-studio = { tag = "lutgen-studio-v0.3.0", alias = "lutgen-gui" }
```

### Repositories with several binaries

Each binary is named after its asset, so `lutgen-cli-v1.1.1-x86_64-unknown-linux-gnu.tar.gz`
gives `lutgen-cli`. binto writes a `binaries` table only when a release really ships more than
one. Once a repository has one, `tag` and `alias` are read per binary, and a `tag` or `alias`
next to `binaries` is ignored.

A pin applies to one binary: pinning `lutgen-studio` leaves `lutgen-cli` on the latest release.
`binto remove` drops only the binary you remove, and the repository's entry goes with its last
binary.

## Sync

```sh
binto sync
```

`binto sync` installs every tool in the manifest that isn't installed yet. A pinned entry
installs its exact tag; the rest install the latest release. Tools that are already installed
are left as they are, and commented-out entries are ignored.

### Pruning

```sh
binto sync --prune        # install what's missing, then offer to remove extras
binto sync --prune -y     # ...without asking
```

`--prune` also works in the other direction: every managed tool the manifest doesn't list is
uninstalled, the same way `binto remove` does it. binto lists those tools and asks first;
`-y` skips the question.

The manifest is the source of truth here, so pruning against an empty manifest removes
everything binto manages.
