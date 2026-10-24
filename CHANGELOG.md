# Changelog

All notable changes to binto are documented here.

## [0.4.0] - 2026-10-24

### 🚀 Features

- *(docs)* Put documentation in a book and cleanup README ([02f04fe](https://github.com/Ardnys/binto/commit/02f04fed99c30c2b8a324c6ef3937db7f5167a85))

### 🐛 Bug Fixes

- Ctrl_c handler runs destructors ([a317c5f](https://github.com/Ardnys/binto/commit/a317c5fb20fa0720674279e6520a44ac7521fd5b))
- Remove pin after successful update ([12d606b](https://github.com/Ardnys/binto/commit/12d606b876b413dbbf6ff5a70a2c345f00ca5d70))
- Handle asset failures per tool ([f658187](https://github.com/Ardnys/binto/commit/f658187ad075a5879bdf47930539746aeb66ca13))
- *(update)* Stop stale snapshot from overwriting updated entries ([4195571](https://github.com/Ardnys/binto/commit/41955712425dc814562ee58dc8f5f26d7e718c25))

### ⚡ Performance

- *(installer)* Bounded concurrent lookups and downloads ([4b777a4](https://github.com/Ardnys/binto/commit/4b777a47c713ce68cf744e98afb901e3639742af))

### 🚜 Refactor

- *(archive)* Unify supported archives with a better abstraction ([2de25d0](https://github.com/Ardnys/binto/commit/2de25d087e6b9e983c9ce07ba292290ac2871f65))
- *(update)* State::mut instead of mut state ([150116c](https://github.com/Ardnys/binto/commit/150116ccb03ec0fcbda7840080a36780e5736c67))
- *(ui)* Callback reporter and prompter for the install pipeline ([03623a5](https://github.com/Ardnys/binto/commit/03623a5a7a68355e149aaff90c2e193fc65e5095))
- *(installer)* One pipeline for install, update and sync ([3a94054](https://github.com/Ardnys/binto/commit/3a9405447a4f1475d00ef20e43dd158bfc983bc7))

### 📚 Documentation

- Fix broken doc links ([0012360](https://github.com/Ardnys/binto/commit/00123604f509ea24aed2a8e7c499d4fd6253485a))

### 🔧 Build and CI

- *(release)* Fail when the tag doesn't match the crate version ([f0394bd](https://github.com/Ardnys/binto/commit/f0394bd6280c1a58029902000d66ecf63ef3fb45))

### ⚙️ Miscellaneous

- *(claude)* Add skills and agents ([bfc174b](https://github.com/Ardnys/binto/commit/bfc174b3ada33ce5bc18ec9fb3291ee3b024f18a))
- *(release)* Add git cliff config and update release skill ([726bb3f](https://github.com/Ardnys/binto/commit/726bb3f0fecd60fa847ff660b3710ec25835b551))
- *(release)* File claude-scoped commits under miscellaneous ([3c3961f](https://github.com/Ardnys/binto/commit/3c3961f0a556e823fa5be28ccdf8fa04e4322879))

## [0.3.0] - 2026-09-30

### 🚀 Features

- Add cross-process file lock for state ([053428a](https://github.com/Ardnys/binto/commit/053428ad12a4505da27db28e2f076d5d55400f17))
- Non-interactive install and libc preference ([c5c3dbc](https://github.com/Ardnys/binto/commit/c5c3dbcf141264a3932211dbe09d7d27957ed9ea))
- *(config)* [**breaking**] Change default installation path ([c354116](https://github.com/Ardnys/binto/commit/c3541167a4ce156ba8b5a271ce854b772383511b))
- Add uninstall command ([4aa2055](https://github.com/Ardnys/binto/commit/4aa205592c401e5b66c4d02b4408ea4fd570546e))
- [**breaking**] Refactor asset matching and add asset matching runner ([996876a](https://github.com/Ardnys/binto/commit/996876a30519c9485f9528dd90fcce2b994b5a6a))
- *(matcher)* Rewrite the asset matching algorithm ([88c2413](https://github.com/Ardnys/binto/commit/88c2413e187686d5d440316a72ef0ef0e132ae0e))
- *(install)* Half-way impl ([25a8d03](https://github.com/Ardnys/binto/commit/25a8d03cd21b72f33166bbd4ecdefe6018174e01))
- Add fact removal ([5f3e5af](https://github.com/Ardnys/binto/commit/5f3e5afc0d35ce3702196f512798a809fbfd5168))
- Integrate stemming to `match_asset` ([1d3cbfc](https://github.com/Ardnys/binto/commit/1d3cbfc1e6c911d26c6bc27d25ffd92783f26a71))
- Keep state by asset stems to install multiples assets of the same repo ([a67a640](https://github.com/Ardnys/binto/commit/a67a640429579306d3859f41d24066554a2b4cd6))
- *(manifest)* [**breaking**] New manifest file supports multiple binaries per repo ([d09e0bb](https://github.com/Ardnys/binto/commit/d09e0bbb14a970ca23addcdbdd01d860cbb2920d))
- *(harness)* Create a TUI to explore runner's output ([1066aa7](https://github.com/Ardnys/binto/commit/1066aa7889a6ebcee3c64f7526a5db13649331d3))
- *(matcher)* Fix stem and version resolution of multiple binaries ([299d873](https://github.com/Ardnys/binto/commit/299d8735e76be1e8fe7348a840c383119751a162))

### 🐛 Bug Fixes

- Handle more SHA checksum cases ([3c89b93](https://github.com/Ardnys/binto/commit/3c89b93d1d31ceee4d985c5d2734d151a28c4b1a))
- Remove unnecessary hashmap and change &String to &str ([9eb8743](https://github.com/Ardnys/binto/commit/9eb8743ea69b96b05a128260300102ad2238ada7))
- Add different checksum name case ([54e6f7d](https://github.com/Ardnys/binto/commit/54e6f7d45ed13cae736bb7cf13f51ee5fece9736))
- *(matcher)* Handle word size and add more rejection extensions ([83b61bc](https://github.com/Ardnys/binto/commit/83b61bc99262ae460726d08564586ffb65892ec2))
- *(clippy)* Default impl for EventHandler ([4099e42](https://github.com/Ardnys/binto/commit/4099e426cd279743a64f6ba1692192f47612cc31))

### ⚙️ Miscellaneous

- Add changelog ([cbc4b70](https://github.com/Ardnys/binto/commit/cbc4b70b5d59f8fa1bc09d983eafaffc538ace76))
- Remove TODO comment about binto removing itself ([87bdfaf](https://github.com/Ardnys/binto/commit/87bdfafb5967f1f5f3b75ecc9bc55c01f8ba7250))

### Other

- Add tracing for testing setup ([345829b](https://github.com/Ardnys/binto/commit/345829b3c63f2f5fea59205ae6b7531fbb0af0d4))

## [0.2.0] - 2026-06-22

### 🚀 Features

- Implement manifest file and version tagging ([abe6371](https://github.com/Ardnys/binto/commit/abe637164680e14a7a4bc2cb2b6ab32794ec0fda))
- Add 'i' alias for install ([aed4e57](https://github.com/Ardnys/binto/commit/aed4e5783f5159e6c12d2995f50c9782a58f2c2a))
- Add "clean" command to clean cache folder ([1703f38](https://github.com/Ardnys/binto/commit/1703f384b22d0a12e7eff179706064a09911ed14))
- Add --to flag for specific install location ([095dc9c](https://github.com/Ardnys/binto/commit/095dc9c8fe03408cbc568cd41f43c880f3d96109))
- Concurrent `cmd_check` ([67cfa4b](https://github.com/Ardnys/binto/commit/67cfa4b751de1ae82e156071e8053d9cb6347531))
- Add -a/--alias for aliasing binaries ([243e57b](https://github.com/Ardnys/binto/commit/243e57b61c1e82800c2b00f54fafec56cf8178f5))
- Add `sync --prune` and preserve manifest structure and comments ([83b8b81](https://github.com/Ardnys/binto/commit/83b8b81d088457d00e6f210d0ce272f694595aa4))
- Add tracing ([95297f6](https://github.com/Ardnys/binto/commit/95297f6a1c0510a49697bdbdda14fdb1f992664e))

### 🐛 Bug Fixes

- Add handler for dialoguer Ctrl-C exits making cursor invisable ([0e0b1b8](https://github.com/Ardnys/binto/commit/0e0b1b88cf88af9e9c88eede990e2f87a0358009))

### ⚡ Performance

- Concurrent downloads on `cmd_sync` ([a0530d4](https://github.com/Ardnys/binto/commit/a0530d4dfd8f59205848db07f236ae0ea7dcb799))

### 🚜 Refactor

- Rewrite installation pipeline ([698208c](https://github.com/Ardnys/binto/commit/698208c53e9c4f82aafa0a31c0b323dd9c64d83c))
- [**breaking**] Rename project ghr -> binto ([1e1beba](https://github.com/Ardnys/binto/commit/1e1beba60405023ddec033c06b1f4e6e8848dcc5))

## [0.1.2] - 2026-06-15

### 🐛 Bug Fixes

- Correct inverted executable check in adopt command ([859882a](https://github.com/Ardnys/binto/commit/859882ac974b6ac4d3f5a3b7028353eaf4a7f9a9))

## [0.1.1] - 2026-06-15

### 🚀 Features

- Concurrent downloads, update fix, refactor ([b8035e8](https://github.com/Ardnys/binto/commit/b8035e87c690943075178031ae280e7a03b0ae43))
- Add confirmation to disable-timer command and update readme ([9930638](https://github.com/Ardnys/binto/commit/99306389392c11f0d0b37fab44b8e0e7a2a116c0))

### 🐛 Bug Fixes

- Use disable variable ([0291690](https://github.com/Ardnys/binto/commit/02916902093f9d14457d33094dbce1a53dd4ff8f))

### Other

- Fix clippy error in CI ([4feb998](https://github.com/Ardnys/binto/commit/4feb99866608b82dcb9e8a64553c8a12264d0c4f))
- Update README.md ([3892bc8](https://github.com/Ardnys/binto/commit/3892bc8fb1a45500e3edaea2d31fbb8c73416510))

## [0.1.0] - 2026-06-12

### Other

- Initial commit ([c4f2490](https://github.com/Ardnys/binto/commit/c4f24904ee5cc0838048d29508c040fb61d2890e))
- Cargo fmt ([dc629ec](https://github.com/Ardnys/binto/commit/dc629ec1f62d9c64b25575d758dbbdf757f5b6d4))
- Cargo clippy --fix ([e3a5822](https://github.com/Ardnys/binto/commit/e3a5822830ffa3dd5fe58e8de38b7fe73e737c78))
- Use rustls instead of native-tls for cross-compilation ([973068c](https://github.com/Ardnys/binto/commit/973068c711f35dbf6743c959401116b94535d6bd))

[0.4.0]: https://github.com/Ardnys/binto/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Ardnys/binto/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Ardnys/binto/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/Ardnys/binto/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Ardnys/binto/compare/v0.1.0...v0.1.1

