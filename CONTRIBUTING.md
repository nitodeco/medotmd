# Contributing

This guide is for humans and AI agents contributing to `medotmd`.

## Prerequisites

- Rust 1.88 or newer
- Cargo

## Local Checks

Run these before opening a pull request:

```sh
cargo fmt --check
cargo check
cargo clippy -- -D warnings
cargo test
cargo build
```

## Test Scope

Integration tests use temporary `HOME` directories and must not touch real agent configuration.

Keep test fixtures local to the test that needs them. If persistent test data becomes necessary, put it in a local `__test__` folder next to the test.

## Versioning

Use semver:

- Patch: bug fixes, documentation, internal cleanup
- Minor: new supported agents or non-breaking CLI flags
- Major: changed command semantics or backup/uninstall behavior

Release tags use `vMAJOR.MINOR.PATCH` and must match `Cargo.toml`.

## Release

Releases are built from git tags by GitHub Actions. The release workflow uploads platform archives and checksums for the install script.
