# Architecture

## High-Level Design

The project is implemented as a Rust CLI with a thin native interop layer inside the same binary.

- Rust handles scanning, hashing, manifest merge/parsing, canonicalization, diffing, state management, publish planning, backend dispatch, WinGetUtil interop, and catalog packaging.
- `WinGetUtil.dll` is still the compatibility backend for the mutable writer, but it is loaded directly from Rust at runtime.
- Static MSIX resources live in the source/template repository under `packaging/`, not in this builder repository.
- Non-Windows packaging uses `makemsix` built from the bundled `msix-packaging` submodule.

## Build Pipeline

1. Scan the repo and hash changed YAML files.
2. Recompute merged manifest snapshots for dirty version directories.
3. Compute:
   - `version_content_sha256`
   - `version_installer_sha256`
   - `published_manifest_sha256`
4. Diff dirty versions against the last successful state.
5. Regenerate only affected package sidecars when the selected format needs them.
6. Apply incremental writer operations or build the published DB directly, depending on the selected backend.
7. Stage a publish tree and emit `source.msix` or `source2.msix`.
8. Commit the staged output and state only after the build succeeds.

## State Store

The state store is a SQLite database that records:

- current file snapshot
- current version snapshot
- current package snapshot
- published file inventory
- per-build version and package change logs

This makes the builder independent from Git commit topology. A run compares current repository state with the last successful published state.

## Hash Model

- `raw_file_hash`: scan cache only.
- `version_content_sha256`: semantic manifest identity used for republish decisions.
- `version_installer_sha256`: installer-affecting identity used for validation routing.
- `published_manifest_sha256`: exact hosted merged manifest bytes.
- package publish hash: exact `versionData.mszyml` bytes.

`Commands`, `Protocols`, and `FileExtensions` are excluded from the installer hash but still participate in the full content hash.

## Output Contract

The output contract is format-dependent:

- `v1`: `source.msix` plus hosted merged manifests
- `v2`: `source2.msix`, `packages/.../versionData.mszyml`, and hosted merged manifests
- `manifests/...`

The core keeps catalog-format handling behind an abstraction so future source formats can be added as new writer implementations.
