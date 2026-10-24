# binto

A user-land binary package manager for GitHub releases. Install, track, and update CLI tools without `sudo`, a system package manager, or compiling from source.

> The name `binto` is derived from both Italian cheese **Bitto Storico** and Japanese lunch **Bentō** (弁当). If that sounds kind of random, you bet your boots it is. All simple and obvious names like ghr, bin, gbin or anything related to GitHub and binaries were taken. I was frustrated trying to find a nice and meaningful name, and at last I thought "I am gonna name it after cheese then" so I started looking up cheese names and **bitto** sounded nice and I converted it to **binto**, which made it similar to **bentō**, arguably more related to package management than cheese. And thus my 2 favorite cuisines found their way into here.

```shell
binto install BurntSushi/ripgrep
binto install https://github.com/sharkdp/bat
binto update --all
```

---

## Install

Download the latest release for your platform from the [releases page](https://github.com/Ardnys/binto/releases/latest), extract, and place the binary on your `PATH`:

```sh
tar -xzf binto-*-x86_64-unknown-linux-gnu.tar.gz
mv binto ~/.local/bin/
```

Then bootstrap binto to manage itself:

```sh
binto adopt ~/.local/bin/binto Ardnys/binto
binto update binto
```

---

## Documentation

The user guide lives in [`docs/`](docs/src/SUMMARY.md) and is built with
[mdBook](https://rust-lang.github.io/mdBook/): `mdbook serve docs` previews it locally.

- [Installing tools](docs/src/guide/installing.md)
- [Updating and pinning](docs/src/guide/updating.md)
- [The manifest and sync](docs/src/guide/manifest.md)
- [Configuration](docs/src/reference/configuration.md)

---

## Roadmap
- [x] aliasing with -a / --alias, for ripgrep for example. should be persisted in manifest as well.
- [x] Concurrent `binto check`
- [x] `binto i` alias for `binto install`
- [x] `binto install --to` command to install to given path
- [x] `binto clean` to clean cache files
- [x] logging / tracing. indicatif has both logging and tracing integrations. logs should be available in a log file. replace `println`s with proper log statements.
- [x] Version pinning with `binto install Ardnys/binto -t v0.1.1`
- [x] **manifest file support**
  - [x] `manifest.toml` alongside config.toml, shows tools and repositories, optional version tags.
  - [x] `binto install` and `binto remove` keeps that file in sync automatically.
  - [x] `binto sync` installs everything in the manifest file that's missing in current state.
- [x] perhaps installing binaries to somewhere related to binto as default, so it's clear what's managed by binto and what is not
- [ ] parse SBOM files

## Contributing
For feature requests and bug reports, please open an issue on GitHub.


## License
binto is licensed under the MIT License.
