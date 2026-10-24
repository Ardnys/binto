# Configuration

binto reads `~/.config/binto/config.toml`, and creates it with the defaults on first run.

```toml
install_dir = "~/.local/share/binto/bin"
include_prereleases = false
check_interval_hours = 24
prefer_libc = "gnu"
# github_token = "ghp_..."
```

| Key | Default | Meaning |
|---|---|---|
| `install_dir` | `~/.local/share/binto/bin` | Where tools are installed. A leading `~` is expanded. `binto install --to` overrides it for one tool. |
| `github_token` | none | A GitHub token for API requests. The `GITHUB_TOKEN` environment variable takes precedence. |
| `include_prereleases` | `false` | List pre-releases in `binto install` and consider them in `binto sync`, as if `--prerelease` were always given. |
| `check_interval_hours` | `24` | After how many hours without a check binto reminds you to run `binto check`. |
| `prefer_libc` | `"gnu"` | `"gnu"` or `"musl"`: which build to prefer when a release ships both. See [How binto picks an asset](../concepts/asset-selection.md). |
| `notify` | `"terminal"` | Reserved for update notifications, which aren't implemented yet. |

## GitHub token

Without a token, GitHub allows 60 API requests an hour; with one, 5,000. Create a token at
<https://github.com/settings/tokens>. Public repositories need no scopes.
