# Installation

Download the archive for your platform from the
[latest release](https://github.com/Ardnys/binto/releases/latest), extract it, and put the
`binto` binary somewhere on your `PATH`:

```sh
tar -xzf binto-*-x86_64-unknown-linux-gnu.tar.gz
mv binto ~/.local/bin/
```

Releases are built for `x86_64` (glibc or musl) and `aarch64` (glibc). On `x86_64`, the
`musl` build is fully static and runs on any distribution.

## Let binto manage itself

Hand the binary you just placed over to binto, so `binto update` keeps binto itself up to date:

```sh
binto adopt ~/.local/bin/binto Ardnys/binto
binto update binto
```

## Put the install directory on your `PATH`

binto installs tools into `~/.local/share/binto/bin` by default, so it's always clear which
binaries binto manages. Add that directory to your `PATH`, for example in `~/.bashrc` or
`~/.zshrc`:

```sh
export PATH="$HOME/.local/share/binto/bin:$PATH"
```

binto warns after an install when the directory it installed into isn't on your `PATH`. To
install somewhere else, change [`install_dir`](reference/configuration.md).
