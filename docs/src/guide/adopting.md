# Adopting existing binaries

```sh
binto adopt ~/.local/bin/fzf junegunn/fzf
binto adopt /usr/local/bin/lazygit jesseduffield/lazygit
```

`binto adopt <path> <repo>` puts a binary that's already on disk under binto's management,
whether it came from a package manager, an install script or a manual download. binto doesn't
move or reinstall it; it records where it is and which repository it comes from, and adds the
repository to the [manifest](manifest.md).

binto doesn't know the adopted binary's version yet. Run `binto update <name>` once: binto
installs the latest release in place, and from then on the tool is updated like any other.

The tool is tracked by its file name, so `~/.local/bin/fzf` becomes `fzf`.
