# Files and directories

| Path | Contents |
|---|---|
| `~/.config/binto/config.toml` | [Configuration](configuration.md) |
| `~/.config/binto/manifest.toml` | The portable list of managed tools; see [The manifest and sync](../guide/manifest.md) |
| `~/.local/share/binto/state.toml` | What's installed on this machine: versions, paths, checksums. Managed by binto; don't edit it. |
| `~/.local/share/binto/bin/` | The default install directory |
| `~/.local/share/binto/logs/` | Daily log files, the last 7 kept |
| `~/.cache/binto/` | Downloads in progress. Emptied after each install; `binto clean` clears leftovers. |

The paths follow the XDG base directories, so `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and
`XDG_CACHE_HOME` move them.

The manifest is the file to keep in your dotfiles. The state file describes only this machine.
