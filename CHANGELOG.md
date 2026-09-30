## [0.3.0] - 2026-09-30

### 🚀 Features

- Add cross-process file lock for state
- Non-interactive install and libc preference
- *(config)* [**breaking**] Change default installation path
- Add uninstall command
- [**breaking**] Refactor asset matching and add asset matching runner
- *(matcher)* Rewrite the asset matching algorithm
- *(install)* Half-way impl
- Add fact removal
- Integrate stemming to `match_asset`
- *(harness)* Create a TUI to explore runner's output
- *(matcher)* Fix stem and version resolution of multiple binaries

### 🐛 Bug Fixes

- Handle more SHA checksum cases
- Remove unnecessary hashmap and change &String to &str
- Add different checksum name case
- *(matcher)* Handle word size and add more rejection extensions
- *(clippy)* Default impl for EventHandler

### ⚙️ Miscellaneous Tasks

- Add changelog
- Remove TODO comment about binto removing itself
## [0.2.0] - 2026-06-22

### 🚀 Features

- Implement manifest file and version tagging
- Add 'i' alias for install
- Add "clean" command to clean cache folder
- Add --to flag for specific install location
- Concurrent `cmd_check`
- Add -a/--alias for aliasing binaries
- Add `sync --prune` and preserve manifest structure and comments
- Add tracing

### 🐛 Bug Fixes

- Add handler for dialoguer Ctrl-C exits making cursor invisable

### ⚡ Performance

- Concurrent downloads on `cmd_sync`

### 🚜 Refactor

- Rewrite installation pipeline
- [**breaking**] Rename project ghr -> binto

### ⚙️ Miscellaneous Tasks

- Bump version to v0.2.0
## [0.1.2] - 2026-06-15

### 🐛 Bug Fixes

- Correct inverted executable check in adopt command

### ⚙️ Miscellaneous Tasks

- Bump version to v0.1.2
## [0.1.1] - 2026-06-15

### 🚀 Features

- Concurrent downloads, update fix, refactor
- Add confirmation to disable-timer command and update readme

### 🐛 Bug Fixes

- Use disable variable

### ⚙️ Miscellaneous Tasks

- Bump version to v0.1.1
## [0.1.0] - 2026-06-12
