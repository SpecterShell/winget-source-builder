# Usage

## Prerequisites

- Windows 10/11 for the full WinGetUtil path and for `v2` sidecar generation.
- `WinGetUtil.dll` next to `winget-source-builder.exe` at runtime. Windows builds provision it automatically from the bundled `winget-cli` submodule.
- `makeappx.exe` from the Windows SDK or `makemsix`. Non-Windows source-checkout builds provision `makemsix` automatically from the bundled `msix-packaging` submodule.
- A manifest repository laid out like WinGet manifests.
- The source repository root should also contain `packaging/`.
- For source-checkout usage: Rust stable. Initialize the bundled submodules with `git submodule update --init --recursive`. `WINGET_CLI_ROOT` or `MSIX_PACKAGING_ROOT` are only needed if you want to override the bundled submodule checkouts.

## Commands

Build a static source tree:

```powershell
cargo run -- build `
  --repo C:\path\to\repo `
  --state C:\path\to\state `
  --out C:\path\to\out `
  --lang en `
  --backend rust `
  --format v2
```

From a packaged artifact:

```powershell
.\winget-source-builder.exe build `
  --repo C:\path\to\repo `
  --state C:\path\to\state `
  --out C:\path\to\out `
  --lang zh-CN `
  --format v2
```

## Environment Variables

- `WINGET_CLI_ROOT`: absolute path to a `winget-cli` checkout for compile-time `WinGetUtil.dll` bootstrap.
- `MSIX_PACKAGING_ROOT`: absolute path to an `msix-packaging` checkout for compile-time `makemsix` bootstrap on Linux/macOS.
- `MAKEAPPX_EXE`: absolute path to `makeappx.exe`.
- `MAKEMSIX_EXE`: absolute path to `makemsix`.
- `WINGET_SOURCE_BUILDER_WORKSPACE_ROOT`: override the workspace root used to find `packaging/`. If `--repo` already points into a source/template repository, the builder will usually infer this automatically.
- `WINGET_SOURCE_BUILDER_LANG`: runtime language for build progress and summary output. Any locale file present under `locales/` can be selected, for example `en` or `zh-CN`.

## Output Tree

- `source.msix` for `--format v1`, or `source2.msix` for `--format v2`.
- `packages/<PackageIdentifier>/<hash8>/versionData.mszyml`: package-level sidecar data for `--format v2`.
- `manifests/...`: content-addressed merged manifests used by the catalog.

## State Tree

- `state.sqlite`: incremental state store.
- `validation-queue.json`: installer revalidation work items.
- `writer/mutable-v1.db` or `writer/mutable-v2.db`: persistent mutable WinGetUtil database, only when using the WinGetUtil backend.
- `staging/`: temporary per-build workspace.

## Incremental Behavior

- Changes are detected from file additions, removals, and content hashes.
- Metadata-only manifest edits republish affected packages without forcing installer revalidation.
- Installer-affecting edits are added to `validation-queue.json`.
- If the previous publish tree under `--out` is missing, update and remove operations cannot replay against prior hosted manifests; use a fresh rebuild flow.
- The Rust backend can package `--format v1` on Linux and macOS through `makemsix`. The Rust `v2` backend remains Windows-only because the package sidecars use MSZIP.
