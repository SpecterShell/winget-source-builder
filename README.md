# winget-source-builder

[简体中文](README.zh-CN.md) | [繁體中文](README.zh-TW.md)

`winget-source-builder` is a Windows-first static WinGet source builder for third-party repositories. It scans a manifest tree, tracks changes by file state instead of Git commits, keeps an internal incremental state store, and publishes a file-based output tree with `source2.msix`, package sidecars, and hosted merged manifests.

User-facing messages are localized through external locale files under `locales/`. Adding a new locale does not require editing Rust source files.

## Features

- File-state incremental builds backed by SQLite state.
- Parallel scan, hashing, merge, and diff stages in Rust.
- Content-addressed hosted manifests and `versionData.mszyml`.
- WinGet-compatible `source2.msix` output.
- Format abstraction in the core so future catalog versions can be added behind a new writer.

## Requirements

- Windows 10/11 for the full build path.
- `WinGetUtil.dll` next to `winget-source-builder.exe` at runtime. Windows builds provision it automatically from the bundled `winget-cli` submodule.
- Windows SDK `makeappx.exe`, or `MAKEAPPX_EXE` pointing to it.
- For source-checkout usage: Rust stable and `git submodule update --init --recursive`.
- The source repository being indexed should contain `packaging/msix/`, for example from `winget-source-template`.

## Quick Start

Build from a source checkout:

```powershell
git submodule update --init --recursive
cargo run -- build `
  --repo C:\path\to\source-repo\manifests `
  --state C:\path\to\builder-state `
  --out C:\path\to\publish-root `
  --lang en `
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

- `source2.msix`
- `packages/<PackageIdentifier>/<hash8>/versionData.mszyml`
- `manifests/...`

State layout:

- `state.sqlite`
- `validation-queue.json`
- `writer/mutable-v2.db`

## GitHub Action

This repository also ships a reusable GitHub Action in [action.yml](https://github.com/SpecterShell/winget-source-builder/blob/main/action.yml). The intended consumer is a source/template repo that contains:

- `manifests/`
- `packaging/msix/`

See `winget-source-template` for the expected layout and workflow pattern.

## Documentation

- [Usage](docs/en/usage.md)
- [Architecture](docs/en/architecture.md)
- [Development and CI](docs/en/development.md)
