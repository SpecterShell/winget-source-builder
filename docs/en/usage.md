# Usage

## Prerequisites

- Windows 10/11 for the full build path.
- `WinGetUtil.dll` next to `winget-source-builder.exe` at runtime. Windows builds provision it automatically from the bundled `winget-cli` submodule.
- Windows SDK `makeappx.exe`, or `MAKEAPPX_EXE`.
- A manifest repository laid out like WinGet manifests.
- The source repository root should also contain `packaging/`.
- For source-checkout usage: Rust stable. Initialize the bundled submodule with `git submodule update --init --recursive`. `WINGET_CLI_ROOT` is only needed if you want to override the bundled `winget-cli` checkout.

## Commands

Build a static source tree:

```powershell
cargo run -- build `
  --repo C:\path\to\repo `
  --state C:\path\to\state `
  --out C:\path\to\out `
  --lang en `
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
- `MAKEAPPX_EXE`: absolute path to `makeappx.exe`.
- `WINGET_SOURCE_BUILDER_WORKSPACE_ROOT`: override the workspace root used to find `packaging/`. If `--repo` already points into a source/template repository, the builder will usually infer this automatically.
- `WINGET_SOURCE_BUILDER_LANG`: runtime language for build progress and summary output. Any locale file present under `locales/` can be selected, for example `en` or `zh-CN`.

## Output Tree

- `source2.msix`: WinGet catalog package for v2 clients.
- `packages/<PackageIdentifier>/<hash8>/versionData.mszyml`: package-level sidecar data.
- `manifests/...`: content-addressed merged manifests used by the catalog.

## State Tree

- `state.sqlite`: incremental state store.
- `validation-queue.json`: installer revalidation work items.
- `writer/mutable-v2.db`: persistent mutable WinGetUtil database.
- `staging/`: temporary per-build workspace.

## Incremental Behavior

- Changes are detected from file additions, removals, and content hashes.
- Metadata-only manifest edits republish affected packages without forcing installer revalidation.
- Installer-affecting edits are added to `validation-queue.json`.
- If the previous publish tree under `--out` is missing, update and remove operations cannot replay against prior hosted manifests; use a fresh rebuild flow.
