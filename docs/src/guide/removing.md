# Removing tools and binto

## Removing a tool

```sh
binto remove rg
binto remove rg -y   # don't ask for confirmation
```

`binto remove <name>` deletes the binary, stops tracking it, and removes it from the
[manifest](manifest.md).

## Clearing the download cache

```sh
binto clean
```

Installs clean up after themselves, but an interrupted or failed run can leave partial
downloads behind in `~/.cache/binto`. `binto clean` removes that directory and reports how much
space it freed.

## Uninstalling binto

```sh
binto uninstall
```

`binto uninstall` always removes binto's state, and lets you choose what else goes:

- Remove installed binaries
- Remove logs
- Remove manifest file
- Remove config file

binto never deletes its own executable as a separate step. The clean way to remove it is to
have binto [manage itself](../installation.md#let-binto-manage-itself), then pick one of:

- **Remove binto entirely:** select *Remove installed binaries*, along with whatever else you
  want gone. The binto binary is deleted with the tools it installed.
- **Keep your tools, drop binto:** leave *Remove installed binaries* unselected. Your binaries,
  binto included, stay on your `PATH` and are yours to manage by hand from then on.

> **Warning:** removing binto's data directory also removes every binary installed in the
> default install directory, `~/.local/share/binto/bin`.
