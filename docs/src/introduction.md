# binto

binto is a user-land binary package manager for GitHub releases. It installs, tracks and
updates command-line tools straight from a project's release assets — no `sudo`, no system
package manager, and no compiling from source.

```sh
binto install BurntSushi/ripgrep
binto install https://github.com/sharkdp/bat
binto update --all
```

For each tool, binto picks the asset built for your machine, verifies its checksum when the
release publishes one, installs the binary, and remembers where it came from so later updates
need no questions. A portable [manifest](guide/manifest.md) lists everything binto manages,
so one `binto sync` reproduces your toolset on another machine.

binto runs on Linux.

## About the name

The name comes from both the Italian cheese **Bitto Storico** and the Japanese lunch
**bentō** (弁当). If that sounds kind of random, you bet your boots it is. All the simple and
obvious names like ghr, bin, gbin, or anything related to GitHub and binaries were taken. Tired
of looking for a meaningful name, I thought "I'm gonna name it after cheese then". **Bitto**
sounded nice, and turning it into **binto** made it close to **bentō**, which is arguably more
related to package management than cheese. And thus my two favourite cuisines found their way
in here.
