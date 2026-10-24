# Environment variables

| Variable | Meaning |
|---|---|
| `GITHUB_TOKEN` | GitHub token for API requests. Takes precedence over `github_token` in the [configuration](configuration.md). |
| `BINTO_CONCURRENT_DOWNLOADS` | How many release lookups and downloads run at once. Default `8`, minimum `1`. |
| `BINTO_LOG` | What the log file records, as [`tracing` filter directives](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html), for example `binto=trace`. `off` disables the log file. Default `debug`. |
| `BINTO_LOG_FORMAT` | `json` prints terminal log events as newline-delimited JSON. |
| `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` | Move binto's [files](files.md) along with the rest of your XDG directories. |
