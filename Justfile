# https://just.systems

run := "cargo run -p binto -- -vv"

build:
    cargo build --workspace

ci:
    cargo fmt --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked

build-release:
    cargo build --workspace --release

binto-list:
    {{run}} list

binto-check:
    {{run}} check

binto-update-all:
    {{run}} update --all

binto-install repo:
    {{run}} i {{repo}}

binto-remove binary:
    {{run}} remove -y {{binary}}

match_default_arch := "x86_64"
match_default_libc := "gnu"

match dataset arch=match_default_arch libc=match_default_libc:
    cargo run --release -p runner -- -d {{dataset}} --arch {{arch}} --libc {{libc}}


insite result:
    cargo run -p insite -- {{result}}
