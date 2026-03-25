# Development and CI

## Local Development

Recommended local checks:

```powershell
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --verbose
```

On Windows, `build.rs` bootstraps `WinGetUtil.dll` next to the built executable at compile time. It uses:

- `WINGET_CLI_ROOT` or the bundled `winget-cli` submodule to run `scripts/build-wingetutil.ps1`

The build no longer accepts a DLL path override, and it no longer scavenges old `WinGetUtil.dll` outputs from sibling checkouts or legacy runtime search paths. Clean-workspace builds are expected to use the bundled submodule or an explicit `WINGET_CLI_ROOT`.

## Test Coverage

- Rust unit tests cover multifile merge and installer-hash filtering.
- A Windows end-to-end test builds the fixture repo in `tests/data/e2e-repo`.
- The end-to-end test skips itself when `WinGetUtil.dll` or `makeappx.exe` is unavailable.
- Runtime i18n tests cover locale normalization, fallback behavior, and translation loading from `locales/`.

## Localization

- User-facing CLI messages are provided by `rust-i18n`.
- Translation strings live under `locales/`, not in Rust source files.
- Adding a new locale is a file-only change unless the program adds new message keys.

## GitHub Actions

The repository ships two workflows:

- `ci.yml`
  - runs `cargo fmt --all --check`
  - runs `cargo clippy --all-targets --all-features -- -D warnings`
  - runs `cargo test --verbose` on Linux, macOS, and Windows
  - checks out submodules on Windows so `build.rs` can provision `WinGetUtil.dll`
  - produces a Windows x64 workflow artifact
- `release.yml`
  - runs on `v*` tags
  - builds the Rust CLI in release mode
  - lets `build.rs` provision `WinGetUtil.dll` during compilation
  - packages a Windows x64 release zip and uploads it to the GitHub release
- `action.yml`
  - exposes this repository as a reusable GitHub Action
  - checks out the action source with submodules
  - builds the Rust CLI on a Windows runner and runs it against a source/template repository

## Release Artifact Layout

The Windows release zip contains:

- `winget-source-builder.exe`
- `WinGetUtil.dll`
- `action.yml`
- `LICENSE`
- `AGENTS.md`
- the multilingual README files
- the `docs/` directory

The Rust binary expects `packaging/` to come from the source/template repository being indexed, not from the builder artifact.
