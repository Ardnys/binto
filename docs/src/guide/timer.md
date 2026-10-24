# Scheduled update checks

```sh
binto setup-timer
binto disable-timer
```

`binto setup-timer` writes a systemd user service and timer to `~/.config/systemd/user/` that
run `binto check` once a day, and offers to enable them right away. A missed run (because the
machine was off) runs at the next boot.

`binto disable-timer` stops, disables and removes both units.

The timer only checks; it never installs anything. See the results with
`journalctl --user -u binto`.
