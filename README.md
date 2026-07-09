# medotmd

`medotmd` is a tiny CLI for people who want one canonical `~/.me/ME.md` identity prompt imported into local coding agents.

It currently supports:

- Codex: `~/.codex/AGENTS.md`
- Claude Code: `~/.claude/CLAUDE.md`
- OpenCode: `~/.config/opencode/AGENTS.md`

It only manages the exact `@/absolute/path/to/.me/ME.md` import line. It does not sync memory, inject prompts at runtime, authenticate agents, or manage project-specific rules.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/nitodeco/medotmd/main/install.sh | sh
```

The installer downloads the latest release for your platform, verifies the checksum, and installs `medotmd` into `~/.local/bin`.

## Setup

```sh
medotmd init
```

`init` creates `~/.me/ME.md` if needed, registers it with detected supported agents, and runs `doctor`.

Only agents with existing parent folders are touched. Existing instruction files are backed up before modification.

## Commands

```sh
medotmd init
medotmd edit
medotmd install
medotmd uninstall
medotmd doctor
medotmd print
```

Use `--dry-run` before changing files:

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
medotmd uninstall --agent opencode
```

## Safety

Before modifying an existing target file, `medotmd` creates a timestamped backup next to it:

```text
AGENTS.md.medotmd.bak-YYYYMMDD-HHMMSS
CLAUDE.md.medotmd.bak-YYYYMMDD-HHMMSS
```

`uninstall` removes only the exact import line managed by `medotmd` and leaves `~/.me/ME.md` untouched.

## Develop

```sh
cargo fmt --check
cargo check
cargo clippy -- -D warnings
cargo test
cargo build
```

See `CONTRIBUTING.md` for contributor and release guidance.
