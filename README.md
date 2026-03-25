# winget-source-builder

[简体中文](README.zh-CN.md) | [繁體中文](README.zh-TW.md)

`winget-source-builder` is a static WinGet source builder for third-party repositories. It scans a manifest tree, tracks changes by file state instead of Git commits, keeps an internal incremental state store, and publishes a file-based output tree with `source.msix` or `source2.msix`, plus any required sidecars and hosted merged manifests.

User-facing messages are localized through external locale files under `locales/`. Adding a new locale does not require editing Rust source files.

## Features

- File-state incremental builds backed by SQLite state.
- Parallel scan, hashing, merge, and diff stages in Rust.
- Content-addressed hosted manifests and `versionData.mszyml`.
- WinGet-compatible `source.msix` and `source2.msix` output.
- Format abstraction in the core so future catalog versions can be added behind a new writer.

## Requirements

- Windows 10/11 for the full WinGetUtil path and for `v2` sidecar generation.
- `WinGetUtil.dll` next to `winget-source-builder.exe` at runtime. Windows builds provision it automatically from the bundled `winget-cli` submodule.
- `makeappx.exe` from the Windows SDK or `makemsix`. Non-Windows builds provision `makemsix` from the bundled `msix-packaging` submodule.
- For source-checkout usage: Rust stable and `git submodule update --init winget-cli msix-packaging`.
- The source repository being indexed should contain `packaging/`, for example from `winget-source-template`.

## Quick Start

Build from a source checkout:

```powershell
git submodule update --init winget-cli msix-packaging
cargo run -- build `
  --repo C:\path\to\source-repo\manifests `
  --state C:\path\to\builder-state `
  --out C:\path\to\publish-root `
  --lang en `
  --backend rust `
  --format v2
```

Run from a packaged Windows artifact:

```powershell
.\winget-source-builder.exe build `
  --repo C:\path\to\source-repo\manifests `
  --state C:\path\to\builder-state `
  --out C:\path\to\publish-root `
  --lang zh-CN `
  --format v2
```

Output layout:

- `source.msix` for `--format v1`, or `source2.msix` for `--format v2`
- `packages/<PackageIdentifier>/<hash8>/versionData.mszyml` for `--format v2`
- `manifests/...`

State layout:

- `state.sqlite`
- `validation-queue.json`
- `writer/mutable-v1.db` or `writer/mutable-v2.db` when using the WinGetUtil backend

`winget-source-template` shows the intended downstream workflow pattern: download a prebuilt builder release with `robinraju/release-downloader`, then run the binary directly in the template repository workflow.

## Documentation

- [Usage](docs/en/usage.md)
- [Architecture](docs/en/architecture.md)
- [Development and CI](docs/en/development.md)
