# Claude Code setup for binto

Project-scoped skills and subagents. They're committed so they evolve with the code.

## Skills — `skills/<name>/SKILL.md`
A skill is instructions Claude loads **into the current conversation** when relevant
(matched against `description`) or when you type `/<name>`.

| Skill | Invoke | Demonstrates |
|---|---|---|
| `ci-check` | auto, or `/ci-check` | `allowed-tools` pre-approves only the cargo commands |
| `matcher-regression` | auto after matcher changes, or `/matcher-regression [prev-dir]` | supporting file (`reference.md`) loaded on demand, `argument-hint` |
| `debug-match` | `/debug-match sharkdp/bat aarch64 musl` | `$ARGUMENTS` substitution |
| `release` | `/release 0.3.0` only | `disable-model-invocation: true` — Claude can't trigger it itself |

## Agents — `agents/<name>.md`
A subagent runs in its **own context window** with its own tools/model and returns only a
summary. Use them for work that would flood the main context or needs a restricted toolset.

| Agent | Tools | Why an agent |
|---|---|---|
| `matcher-analyst` | read-only + Bash, sonnet | results JSONL is huge; only the pattern summary comes back |
| `rust-reviewer` | read-only, inherits model | fresh eyes on the diff, can't "fix" things silently |
| `docs-sync` | can Edit, sonnet | example of a writing agent scoped to Markdown only |

Invoke by asking: "use the matcher-analyst agent on results-x86_64-gnu.jsonl", or let
Claude delegate on its own ("use proactively" in the description encourages that).
Manage them with `/agents`.

## Refining
- Triggering is driven by `description`. If a skill/agent fires too often or never, edit that first.
- Keep `SKILL.md` short; move long reference material to sibling files and link it.
- Tighten `tools` on agents as you learn what they actually need.
- Composition: skills can mention agents (matcher-regression → matcher-analyst) and other
  skills (release → ci-check).
