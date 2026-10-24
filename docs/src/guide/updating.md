# Updating and pinning

## Checking for updates

```sh
binto check
binto check --json | jq '.[] | select(.update_available)'
```

`binto check` looks up the latest release of every managed tool and installs nothing. It exits
with code `1` when any update is available, so scripts can act on it. Pinned tools are skipped.

`binto list` shows every managed tool with its installed version and when it was last checked.
When a tool hasn't been checked for longer than
[`check_interval_hours`](../reference/configuration.md), binto reminds you to run
`binto check`.

## Updating

```sh
binto update rg
binto update --all
```

An update installs the newest release over the old one, in the same directory and under the
same name. binto remembers which asset it picked the first time, so an update doesn't ask
again unless the release changed shape.

`binto update --all` updates every tool that isn't pinned. Lookups and downloads run
concurrently; see [`BINTO_CONCURRENT_DOWNLOADS`](../reference/environment.md). If one tool
fails, the others still update, and binto exits with an error at the end.

## Pinning

Installing with `-t <tag>` pins a tool to that release. A pin is a lock: `binto update` and
`binto check` leave the tool alone.

- **Move a pin:** `binto install <repo> -t <new-tag>` reinstalls the tool at the new tag and
  updates the pin.
- **Drop a pin:** `binto update <name> --force` updates the tool to the latest release and
  clears the pin, so it tracks the latest release again.

`--force` only works on a named tool. With `--all`, pinned tools always stay where they are.

Pins live in the [manifest](manifest.md), so you can also pin or unpin a tool by editing it.
