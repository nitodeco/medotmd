# medotmd CLI Reference

Use this reference only when the user asks to install, initialize, verify, or manage medotmd.

## Purpose

`medotmd` manages one canonical local identity file:

```text
~/.me/ME.md
```

It registers that file with supported local coding agents by adding an import line to each agent's instruction file.

## Supported agents

- Codex: `~/.codex/AGENTS.md`
- Claude Code: `~/.claude/CLAUDE.md`
- OpenCode: `~/.config/opencode/AGENTS.md`

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/nitodeco/medotmd/main/install.sh | sh
```

## First run

```sh
medotmd init
```

`init` creates `~/.me/ME.md` when needed, registers detected supported agents, and runs `doctor`.

## Common commands

```sh
medotmd init
medotmd edit
medotmd install
medotmd uninstall
medotmd doctor
medotmd print
```

## Safe checks

Preview changes:

```sh
medotmd init --dry-run
medotmd install --dry-run
medotmd uninstall --dry-run
```

Target one agent:

```sh
medotmd install --agent codex
medotmd install --agent claude
medotmd install --agent opencode
medotmd doctor --agent claude
```

## Agent guidance

- Use `medotmd print` to inspect the current profile.
- Use `medotmd doctor` to verify registration state.
- Do not run `uninstall` unless the user explicitly asks.
- Ask before overwriting or substantially replacing an existing `~/.me/ME.md`.
