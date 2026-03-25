# AGENTS

## Repository Layout

- `src/`: Rust CLI, manifest logic, i18n integration, state store, and build orchestration.
- `locales/`: external translation files consumed by `rust-i18n`.
- `winget-cli/`: git submodule used to build `WinGetUtil.dll` during Windows builds.
- `action.yml`: reusable GitHub Action entrypoint for source/template repositories.
- `scripts/`: local and CI bootstrap scripts, including native WinGetUtil provisioning.
- `tests/data/e2e-repo/`: minimal fixture repo for end-to-end testing, including template-style `packaging/msix/`.
- `docs/`: multilingual project documentation.
- `.github/workflows/`: CI and release automation.

## Validation

- Standard local validation commands:
  `cargo fmt --all`
  `cargo clippy --all-targets --all-features -- -D warnings`
  `cargo test --verbose`
- When the Windows build path changes, also validate a Windows release build:
  `cargo build --release --target x86_64-pc-windows-msvc`
- For workflow or packaging changes, prefer a real end-to-end builder run against the fixture repo or a template-style source repo in addition to unit tests.

## Status

- V1 targets a Windows-first static publish tree containing `source2.msix`, hosted merged manifests, and compressed per-package `packages/<PackageIdentifier>/<hash8>/versionData.mszyml`.
- The valid preindexed source contract is: signed `source.msix` or `source2.msix` carrying `Public/index.db`, with hosted manifest payloads and per-package `versionData.mszyml` sidecars alongside it.
- V1 records validation requirements only. Installer execution belongs to a later standalone validation pipeline.
- `source2.msix` is the only source package format in scope for v1.
- Runtime i18n is backed by `rust-i18n` and external locale files under `locales/`. New locales can be added without editing Rust source files.
- The builder repository now ships a reusable GitHub Action, while source branding and MSIX resources live in the separate template/source repository.

## Known Limitations

- V1 is Windows-first and `source2.msix`-only.
- V1 queues validation requirements but does not execute installer validation.
- If WinGetUtil remains the writer, final publish work is still partly `O(total packages)` because published package tables are rebuilt during packaging.
- One mutable index cannot be meaningfully parallelized. Parallelism applies around scan, parse and merge, hashing, diffing, staging, and validation scheduling.
- The current incremental remove and update path still depends on the previous published `--out` tree being present, because old hosted manifests are needed for replay.

## Related Projects And References

- `winget-cli` is the main compatibility reference.
  `WinGetUtil.h` and `IWinGetSQLiteIndex.cs` expose the real incremental writer surface: manifest add, update, remove, packaging prep, and manifest-validation options.
  Schema v2 code confirms `PrepareForPackaging()` is destructive to the mutable index, so this project must keep a persistent mutable DB and package from a staged copy.
  `SQLiteIndex.cpp` also shows `AddManifest(path, relativePath)` hashes the parsed manifest stream, which is why this builder feeds deterministic hosted merged manifest files instead of raw multi-file directories.
- `winget-pkgs` is both the scale target and a validation reference.
  The repo includes public tooling such as `Tools/SandboxTest.ps1`, which is relevant for the later sandbox or VM validation pipeline.
  On the inspected local snapshot at commit `2e11fbf606b`, `manifests/` contained about `482,845` YAML files across `125,409` version directories and `11,985` package directories, so performance planning should be framed around package and version scale rather than rough manifest-count guesses.
- `winget-source` is the best reference for published artifact layout.
  Inspected hosted manifest files are merged manifests with hash-like or hash-prefixed names, including extensionless names like `aaa7` and names like `5a5a-0xGingi.Browser.yaml`.
  That supports generated `manifest_relpath` values and content-addressed or hash-prefixed manifest naming for cache busting instead of reusing source-repo filenames.
- `winget-extras` is useful as an Actions-based validation reference, not as an indexing reference.
  Its repo contains a real CI validation path via `.github/workflows/validate.yml` and `validate.ps1`.
  Its publish workflow still merges all manifests into `%TEMP%\\manifests` and runs `IndexCreationTool.exe -f source.json` over the full set, so it remains a rebuild-style flow with no diffing or persisted state layer.

## Architecture Decisions

- Keep the core pipeline backend-agnostic and package-centric.
- Rust owns scan, WinGet-compatible merge/parsing, canonicalization, hashing, diffing, state management, validation scheduling, staging, publish planning, direct WinGetUtil interop, and `makeappx` orchestration.
- Keep the Windows boundary thin even though it now lives inside Rust. A future custom writer should still be able to replace WinGetUtil-facing code without changing scan, diff, or state logic.
- Deterministic hosted merged manifests are the single source of truth for published manifest bytes and for WinGetUtil ingestion.
- The builder consumes `packaging/msix/` from the source/template repository. Branding and Appx metadata do not belong in the builder repository.
- Source format support should stay behind the writer and publisher boundary so future source versions can be added without rewriting the core pipeline.

## State Store And Hash Design

- Treat the state store as a build ledger, not as the published source.
- Keep `state.sqlite` separate from the persistent mutable WinGetUtil DB and from the staged or published `index.db`.
- Core tables should cover current snapshots, candidate build diffs, published artifact bookkeeping, and validation cache or state.
- Track distinct identities:
  `raw_file_hash`, `version_content_sha256`, `version_installer_sha256`, `published_manifest_sha256`, and `package_publish_sha256`.
- `ManifestSHA256Hash` in the index is the hosted manifest file-byte hash, not the semantic manifest hash, so both must be stored explicitly.
- Generate `manifest_relpath` instead of copying repo filenames. Use content-addressed or hash-prefixed hosted merged manifest paths for cache busting and stable publish identity.
- Build `package_publish_sha256` from the exact compressed `versionData.mszyml` bytes. It drives `packages/<PackageIdentifier>/<hash8>/versionData.mszyml`.
- Canonicalize from the merged manifest object before locale application. Do not hash raw directory iteration order, raw merged YAML bytes, or locale-applied views.
- Mirror WinGet ordering: preserve sequence order where meaningful, sort set-like collections for stability, and order package versions by channel ascending then version descending.
- Exclude `Commands`, `Protocols`, and `FileExtensions` from the installer hash, but keep them in the full content hash.

## Manifest Compatibility Rules

- Treat `PackageVersion` as textual version identity, not as a YAML number.
  Preserve the exact manifest or directory text for values like `3.0` and `3.10`; they must not collapse to `3` or `3.1` during parsing, merge, hashing, or hosted-manifest generation.
- ARP `DisplayVersion` collision handling is package-wide and version-aware.
  When multiple current versions of the same package declare the same `DisplayVersion`, keep it only on the manifest with the highest `PackageVersion` and strip it from the lower versions.
- ARP collision handling must look at both root-level and installer-level `AppsAndFeaturesEntries`.
  Some manifests, such as `Unity.UnityHub`, declare `DisplayVersion` at the merged manifest root rather than inside individual installers.
- ARP collision policy can create synthetic manifest updates.
  When the winning version changes, the builder may need to republish versions whose source files did not change so the published manifests still follow the “highest version only” rule.

## Building Plan

1. Ship the WinGetUtil-backed writer first for compatibility.
   The first shipping backend should wrap Microsoft’s writer rather than reimplement it.
2. Publish through a build-scoped staging tree and atomic promotion.
   Never mutate current state or published output until the candidate build succeeds.
3. Keep the builder focused on indexing, not installer execution.
   Validation should be queued now and executed later in a separate Windows Sandbox or VM-oriented pipeline.
4. Preserve the upgrade path for future source formats.
   `source2.msix` is enough for v1, but the core should not hard-code the source format boundary.
5. Revisit a custom writer only when the compatibility tradeoff becomes justified.
   The asymptotic upside is real, but correctness parity with WinGet is the hard part.

## Milestones

- Milestone 0: architecture spike
  Lock the source contract, writer boundary, direct WinGetUtil interop approach, and state/hash model.
- Milestone 1: state engine
  Implement file-state scan, version and package snapshots, candidate build journal, and hash-based diffing.
- Milestone 2: WinGet-compatible writer
  Add persistent mutable DB handling, incremental add/update/remove, staged `PrepareForPackaging()`, and `source2.msix` creation.
- Milestone 3: staged publish tree
  Add content-addressed hosted manifests, `versionData.mszyml` sidecars, exact delete sets, and atomic promotion.
- Milestone 4: validation pipeline
  Consume queued installer validation work outside the builder.
- Milestone 5: custom writer and future source formats
  Preserve a clean upgrade path without disturbing the core pipeline.

## Lessons Learned

- File-state tracking is the right abstraction. Git commit shape is not.
- Keep the last successful state immutable until staged publish succeeds. Malformed manifests must fail the candidate build without poisoning current state.
- Metadata-only edits still change published package artifacts. “No installer retest” does not mean “no republish.”
- Feed WinGetUtil deterministic hosted merged manifests, not raw multifile directories, so published manifest hashes stay explicit and stable.
- Build or copy `WinGetUtil.dll` at compile time instead of trying to provision it at runtime.
  `build.rs` bootstraps `WinGetUtil.dll` from `WINGET_CLI_ROOT` or the bundled `winget-cli` submodule via `scripts/build-wingetutil.ps1`, and places it next to the built executable.
  Clean-workspace builds must not rely on DLL path overrides, sibling-checkout outputs, or legacy runtime search paths.
- The biggest performance win comes from true incremental design and parallel preprocessing, not from Rust replacing C# by itself.
- A custom writer can improve small-delta asymptotics, but exact WinGet compatibility becomes the hard part.
- Keep MSIX static resources out of Rust source files.
  `packaging/msix/` should stay in the source/template repository so Appx manifest and image updates do not require rebuilding the action or touching builder internals.
- The Windows boundary should stay thin. Rust is a good fit for parallel scan and diff work, and direct FFI keeps the WinGetUtil path simpler than a separate wrapper executable.
- CI must not assume a fully provisioned Windows indexing environment. The end-to-end path needs to skip itself cleanly when `WinGetUtil.dll` or `makeappx.exe` is unavailable.
- GitHub-hosted Windows builds should be treated as `windows-2025` builds, not a vague `windows-latest` target.
  The workflows and reusable action now add `VCPKG_INSTALLATION_ROOT` to `PATH` explicitly before building `winget-cli`, instead of assuming `vcpkg.exe` is already resolvable.
