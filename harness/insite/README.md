# insite

Insite is a TUI to explore the `binto` asset matching algorithm doing its magic on an asset dataset.
It's used by `binto` developers to improve the asset matching algorithm and find edge cases and bugs.

To run insite, you have to first run the `runner` on an asset dataset. A dataset could be created by gh-release-data-script.

```bash
cargo run --release -p runner -- -d path/to/dataset -o results.jsonl
```

Then results.jsonl could be investigated by `insite`. 

```bash
cargo run --release -p insite -- results.jsonl
```

It shows auto selected assets, assets that need interaction and no matches; which assets were rejected by what and how the stem was extracted etc.
Also supports searching one needs to search for a repository.
