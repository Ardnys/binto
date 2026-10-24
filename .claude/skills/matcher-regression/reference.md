# jq recipes for runner results

All taken from `harness/runner/README.md`. `F` = a results JSONL file.

```sh
# outcome counts
jq -r .outcome F | sort | uniq -c

# per-repo diff between two runs (join, NOT jq -s)
cmp() {   # cmp before.jsonl after.jsonl
  paste <(jq -r '"\(.repo)|\(.outcome)|\(.verdict.selected.name // "-")"' "$1" | sort) \
        <(jq -r '"\(.repo)|\(.outcome)|\(.verdict.selected.name // "-")"' "$2" | sort) \
  | awk -F'\t' '{split($1,a,"|"); split($2,b,"|");
      if (a[2]!=b[2])                  printf "%-28s %s -> %s\n", a[1], a[2], b[2];
      else if (a[3]!=b[3])             printf "%-28s PICK %s -> %s\n", a[1], a[3], b[3] }'
}

# repos installing a libc they did not ask for
jq -r 'select(.verdict.selected.notes[]?.note=="fallback") | .repo' F

# winner names no architecture
jq -r 'select([.verdict.selected.notes[]?.dimension] | contains(["arch"])) | .repo' F

# what the hard filters removed, dataset-wide
jq -r '.trace[]? | select(.message=="asset rejected") | .reason' F | sort | uniq -c | sort -rn

# auto-selected with no checksum found
jq -r 'select(.outcome=="auto_selected" and .verdict.checksum==null) | .repo' F
```
