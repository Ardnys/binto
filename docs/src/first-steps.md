# First steps

Install a tool by its GitHub repository:

```sh
binto install sharkdp/fd
```

binto shows the release list to pick from, chooses the asset built for your machine, and
installs `fd`. Pass `-y` to skip the questions and take the latest release.

See what binto manages:

```sh
binto list
```

Check for and install updates:

```sh
binto check
binto update --all
```

Everything you install is recorded in `~/.config/binto/manifest.toml`. Copy that file to
another machine and run `binto sync` to install the same tools there. See
[The manifest and sync](guide/manifest.md).

Run `binto --help`, or `binto <command> --help`, for every flag.
