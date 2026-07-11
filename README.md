# medotmd

`medotmd` is a tiny CLI for people who want one canonical `~/.me/ME.md` identity prompt imported into local coding agents.

It currently supports:

- Codex: `~/.codex/AGENTS.md`
- Claude Code: `~/.claude/CLAUDE.md`
- OpenCode: `~/.config/opencode/{config.json,opencode.json,opencode.jsonc}`

It only manages the exact absolute imports for `~/.me/AGENT.md` and `~/.me/ME.md`. It does not sync memory, inject prompts at runtime, authenticate agents, or manage project-specific rules.

## Install

```sh
installer_dir="$(mktemp -d)"
trap 'rm -rf "$installer_dir"' EXIT

curl -fsSL \
  https://github.com/nitodeco/medotmd/releases/latest/download/install.sh \
  -o "$installer_dir/install.sh"
curl -fsSL \
  https://github.com/nitodeco/medotmd/releases/latest/download/install.sh.sig \
  -o "$installer_dir/install.sh.sig"

cat > "$installer_dir/release-signing-public-key.pem" <<'EOF'
-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEA+vCqLtCTcrwVqJFp+zb6+KUpIZqJi+5VcSu/L/b2+94=
-----END PUBLIC KEY-----
EOF

openssl pkeyutl -verify -rawin -pubin \
  -inkey "$installer_dir/release-signing-public-key.pem" \
  -in "$installer_dir/install.sh" \
  -sigfile "$installer_dir/install.sh.sig" >/dev/null
sh "$installer_dir/install.sh"
```

This requires `curl`, `openssl`, `tar`, and `sh`. The bootstrap downloads a signed installer asset from GitHub Releases and verifies its detached Ed25519 signature before executing it. The installer downloads the latest signed release for your platform, verifies its signed manifest, checksum, and archive signature, then installs `medotmd` into `~/.local/bin`.

To install a specific release, replace both `latest` URL segments with its release tag, such as `v0.5.0`, and run `MEDOTMD_VERSION=v0.5.0 sh "$installer_dir/install.sh"` as the final command. Releases published before the signed-installer change do not have these assets.

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
medotmd update
```

`medotmd update` installs a newer stable GitHub Release only after verifying its signed manifest and release archive. It leaves the current binary in place when no update is available.

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

`uninstall` removes only the exact imports managed by `medotmd` and leaves `~/.me/ME.md` untouched.

## Develop

```sh
cargo fmt --check
cargo check
cargo clippy -- -D warnings
cargo test
cargo build
```

See `CONTRIBUTING.md` for contributor and release guidance.
