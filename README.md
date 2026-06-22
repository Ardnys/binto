# binto

A user-land binary package manager for GitHub releases. Install, track, and update CLI tools without `sudo`, a system package manager, or compiling from source.

> The name `binto` is derived from both Italian cheese **Bitto Storico** and Japanese lunch **Bentō** (弁当). If that sounds kind of random, you bet your boots it is. All simple and obvious names like ghr, bin, gbin or anything related to GitHub and binaries were taken. I was frustrated trying to find a nice and meaningful name, and at last I thought "I am gonna name it after cheese then" so I started looking up cheese names and **bitto** sounded nice and I converted it to **binto**, which made it similar to **bentō**, arguably more related to package management than cheese. And thus my 2 favorite cuisines found their way into here.

```
binto install BurntSushi/ripgrep
binto install https://github.com/sharkdp/bat
binto update --all
```

---

## Install

Download the latest release for your platform from the [releases page](../../releases/latest), extract, and place the binary on your `PATH`:

```sh
tar -xzf binto-*-x86_64-unknown-linux-gnu.tar.gz
mv binto ~/.local/bin/
```

Then bootstrap binto to manage itself:

```sh
binto adopt ~/.local/bin/binto Ardnys/binto
binto update binto
```

---

## Usage

### `binto install <repo>`

Fetch releases for a GitHub repository, pick a release and asset interactively, download, verify checksum, and install to `~/.local/bin`.

```sh
binto install BurntSushi/ripgrep
binto i https://github.com/cli/cli      # aliased to i, to save some keys
binto i sharkdp/fd --prerelease         # include pre-releases
binto i sharkdp/bat -t v0.24.0          # pin to a specific release tag
binto i junegunn/fzf --to ~/bin         # install into a specific directory
binto i BurntSushi/ripgrep -a rg        # install under a custom binary name
```

`<repo>` accepts `owner/repo` or any `github.com` URL (with or without scheme, trailing paths are ignored). `binto i` is a shorthand alias for `binto install`.

Pass `--to <path>` to install into a directory other than the configured `install_dir` (a leading `~` is expanded). The choice is recorded in the tool's install path, so later `binto update`s reinstall it there too. It's a local override and is not written to the manifest.

Pass `-a/--alias <name>` to install the binary under a custom name instead of the repo-derived default. For example, `binto install BurntSushi/ripgrep -a rg` installs `rg`. The alias becomes the installed filename and the name binto tracks it by (`binto update rg`, `binto remove rg`), and is recorded in the manifest so `binto sync` reproduces it on another machine. The binary *inside* the archive is still auto-detected as usual; the alias only renames the installed file.

Pass `-t/--tag <tag>` to install (and pin) an exact release instead of picking interactively. A pinned tool is **locked**: `binto update` skips it until you explicitly unpin it with `binto update <name> --force` (see below). To move a pin to a different tag, re-run `binto install <repo> -t <newtag>` on the already-managed tool, which reinstalls at that tag and updates the pin in place. Every install records the tool in the [manifest](#manifest).

### `binto update [name] [--all] [-f/--force]`

Update one or all managed tools to their latest release. Pinned tools are skipped by default. Use `-f/--force` on a named tool to update it to the latest release anyway, which clears its pin (the tool tracks latest again afterwards). `--force` has no effect with `--all` — pinned tools stay locked there; name the tool to force it.

```sh
binto update ripgrep
binto update --all
binto update bat --force   # update a pinned tool to latest and clear its pin
```

### `binto check [--json]`

Check for available updates without installing anything. Checks run concurrently, and pinned tools are skipped. Exits with code `1` if any updates are available, making it useful in scripts.

```sh
binto check
binto check --json | jq '.[] | select(.update_available)'
```

### `binto list [--json]`

List all managed tools with their installed version and last-checked timestamp.

```sh
binto list
binto list --json
```

### `binto adopt <path> <repo>`

Register a binary that is already on disk (installed by a package manager, curl script, etc.) under binto management without moving or reinstalling it. Subsequent `binto update` calls will update it in place.

```sh
binto adopt ~/.local/bin/fzf junegunn/fzf
binto adopt /usr/local/bin/lazygit jesseduffield/lazygit
```

### `binto remove [-y] <name>`

Uninstall a binary and remove it from binto state. `-y` to skip confirmation prompt

```sh
binto remove ripgrep
```

### `binto sync`

Install every tool listed in the [manifest](#manifest) that is missing from local state. Pinned entries install their exact tag; the rest install the latest release. Tools already installed are left untouched. This is how you reproduce your toolset on a new machine after copying over `manifest.toml`.

```sh
binto sync
```

Pass `--prune` to also make local state match the manifest in the other direction: any managed tool whose repo is **not** in the manifest is uninstalled (its binary deleted and the tool untracked, mirroring `binto remove`). The tools to be removed are listed and confirmed first; add `-y/--yes` to skip the prompt for unattended runs. The manifest is the source of truth, so pruning against an empty manifest removes everything binto manages.

```sh
binto sync --prune        # install missing, then offer to remove extras
binto sync --prune -y     # ...without the confirmation prompt
```

### `binto clean`

Remove binto's download cache at `~/.cache/binto`. Installs already clean up after themselves, but interrupted or failed runs can leave partial downloads and extraction directories behind. 

```sh
binto clean
```

### `binto setup-timer`

Write a systemd user service and timer that runs `binto check` on a schedule and optionally enable it immediately.

```sh
binto setup-timer
```

### `binto disable-timer`

Disable and remove the binto timer unit created by `binto setup-timer`.

```sh
binto disable-timer
```

---

## Configuration

Config is stored at `~/.config/binto/config.toml` and created with defaults on first run.

```toml
install_dir = "~/.local/bin"
github_token = ""           # or set GITHUB_TOKEN env var
include_prereleases = false
check_interval_hours = 24
notify = "terminal"         # "terminal" | "desktop" | "none"
# notifications have not been implemented yet though
```

**`GITHUB_TOKEN`** — unauthenticated requests are limited to 60/hour. A token raises this to 5000/hour. Create one at <https://github.com/settings/tokens> (no scopes needed for public repos).

---

## Logging

Every run writes a detailed, rotating log to `~/.local/share/binto/logs/` (daily files, the last 7 kept). The install pipeline is instrumented with spans, so a failed install is traceable to the exact phase. 

Terminal verbosity is controlled per-invocation; the file log stays at `debug` regardless so there's always a post-mortem trail:

```sh
binto -v update --all     # show debug detail on the terminal (-vv for trace)
binto -q sync             # only warnings and errors on the terminal
```

`binto_LOG` overrides the **file** log filter using [`tracing` directives](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) (e.g. `binto_LOG=binto=trace`); `binto_LOG=off` disables the file log entirely. Human-facing messages go to stderr, leaving stdout clean for piping `--json` output.

---

## How asset selection works

binto filters out checksums, source archives, Windows/macOS assets, `.deb`/`.rpm` packages, then scores remaining assets by:

- Architecture match (`x86_64`, `aarch64`, `armv7`, `i686` and common synonyms like `amd64`, `arm64`)
- `linux` keyword presence
- libc preference (`gnu` > `musl`)
- Format preference (raw binary > tar > zip > AppImage)

If the top candidate's score is sufficiently ahead of the second, it is selected automatically. Otherwise an interactive picker is shown. The selected asset's name pattern is saved so future updates skip the picker entirely.

---

## State files

| Path | Purpose |
|------|---------|
| `~/.config/binto/config.toml` | User configuration |
| `~/.config/binto/manifest.toml` | Declarative, portable list of managed tools (repo + optional pinned tag) |
| `~/.local/share/binto/state.toml` | Installed tools, versions, checksums, ETags |
| `~/.local/share/binto/logs/` | Rotating debug logs (one per day, last 7 kept) |
| `~/.cache/binto/` | Download cache (cleaned after each install; `binto clean` clears any leftovers) |

---

## Manifest

`~/.config/binto/manifest.toml` is a declarative, portable list of the tools binto manages. It contains only essential informations about tools and preferences, so you can commit it to your dotfiles and replay it on another machine to get the same setup.

`binto install`, `binto remove`, and `binto adopt` keep it in sync automatically. You can also hand-edit it, and your edits survive: binto rewrites only the one entry it's changing, so **comments, ordering, blank lines, and commented-out entries are preserved** across automatic updates.

```toml
# You can add this to your dotfiles
[[tools]]
repo = "BurntSushi/ripgrep"
alias = "rg"         # optionally install/track the binary under this name

[[tools]]
repo = "sharkdp/bat"
tag = "v0.24.0"      # optionally pin a tool to a specific release tag

# comment a block out to keep it around without syncing it
# [[tools]]
# repo = "junegunn/fzf"
```

Comment out a whole `[[tools]]` block to disable an entry without losing it: `binto sync` (and `--prune`) ignore commented-out tools, and binto won't clobber the comment next time it edits the file. An inline comment on a `tag` you later re-pin via binto is kept too.

Run `binto sync` to install everything in the manifest that isn't installed yet. A `tag` both selects the version `sync` installs and locks the tool so `binto update` skips it.

## Roadmap
- [x] aliasing with -a / --alias, for ripgrep for example. should be persisted in manifest as well.
- [x] Concurrent `binto check`
- [x] `binto i` alias for `binto install`
- [x] `binto install --to` command to install to given path
- [x] `binto clean` to clean cache files
- [x] logging / tracing. indicatif has both logging and tracing integrations. logs should be available in a log file. replace `println`s with proper log statements.
- [x] Version pinning with `binto install Ardnys/binto -t v0.1.1`
- [x] **manifest file support**
  - [x] `manifest.toml` alongside config.toml, shows tools and repositories, optional version tags.
  - [x] `binto install` and `binto remove` keeps that file in sync automatically.
  - [x] `binto sync` installs everything in the manifest file that's missing in current state.
- [ ] perhaps installing binaries to somewhere related to binto as default, so it's clear what's managed by binto and what is not

## Contributing
For feature requests and bug reports, please open an issue on GitHub.


## License
binto is licensed under the MIT License.
