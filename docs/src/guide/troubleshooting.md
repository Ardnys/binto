# Logs and troubleshooting

## Terminal output

```sh
binto -v update --all     # show debug detail (-vv for trace)
binto -q sync             # only warnings and errors
```

Messages for you go to stderr, so stdout stays clean for piping `--json` output.

## Log files

Every run writes a detailed log to `~/.local/share/binto/logs/`, one file per day, keeping the
last 7. The file log records at `debug` level no matter what `-v` or `-q` say, so a failed
install always leaves a trail of what happened.

Set `BINTO_LOG` to change what the file log records, using
[`tracing` filter directives](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html),
for example `BINTO_LOG=binto=trace`. `BINTO_LOG=off` turns the file log off.

## Rate limits

Without a token, GitHub allows 60 API requests an hour. binto warns when you're close to the
limit. Set a [`GITHUB_TOKEN`](../reference/environment.md) to raise it to 5,000.
