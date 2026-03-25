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

On Linux and macOS, `build.rs` bootstraps `makemsix` next to the built executable at compile time. It uses:

- `MSIX_PACKAGING_ROOT` or the bundled `msix-packaging` submodule to run `scripts/build-makemsix.sh`

The build no longer accepts a DLL path override, and it no longer scavenges old `WinGetUtil.dll` outputs from sibling checkouts or legacy runtime search paths. Clean-workspace builds are expected to use the bundled submodule or an explicit `WINGET_CLI_ROOT`.

## Test Coverage

- Rust unit tests cover multifile merge and installer-hash filtering.
- Windows end-to-end tests build the fixture repo in `tests/data/e2e-repo`.
- A Rust `v1` end-to-end test can also run on Linux and macOS when `makemsix` is available.
- Backend-specific end-to-end tests skip themselves when the required packaging/runtime dependencies are unavailable.
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
  - checks out first-level submodules in test/build jobs so `build.rs` can provision `WinGetUtil.dll` and `makemsix`
  - produces Linux x86_64, macOS, Windows x86_64, and Windows aarch64 workflow artifacts
- `release.yml`
  - runs on `v*` tags
  - builds the Rust CLI in release mode
  - lets `build.rs` provision `WinGetUtil.dll` during compilation
  - packages Linux x86_64, macOS, Windows x86_64, and Windows aarch64 release artifacts and uploads them to the GitHub release

Downstream repositories are expected to download the published Windows release artifact directly in their own workflows, for example with `robinraju/release-downloader`, instead of using a reusable action from this repository.

## Release Artifact Layout

The Windows release zip contains:

- `winget-source-builder.exe`
- `WinGetUtil.dll`

The Rust binary expects `packaging/` to come from the source/template repository being indexed, not from the builder artifact.
