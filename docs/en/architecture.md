# Architecture

This document explains how `winget-source-builder` works under the hood. Understanding these concepts helps you troubleshoot issues and make the most of incremental builds.

## Table of Contents

- [High-Level Design](#high-level-design)
- [Build Pipeline](#build-pipeline)
- [State Management](#state-management)
- [Hash Model](#hash-model)
- [Output Formats](#output-formats)

## High-Level Design

`winget-source-builder` is a Rust CLI with a thin native interop layer. The design prioritizes:

- **Incremental processing** — Only changed content is reprocessed
- **State isolation** — Build state is independent of Git history
- **Backend abstraction** — Support for multiple index writing strategies
- **Cross-platform** — Works on Windows, Linux, and macOS

```
┌─────────────────────────────────────────────────────────────┐
│                      Build Pipeline                          │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────┐  │
│  │   Manifest  │───▶│    Scan     │───▶│  Compute Hashes │  │
│  │ Repository  │    │   & Parse   │    │  (SHA256)       │  │
│  └─────────────┘    └─────────────┘    └─────────────────┘  │
│                                                 │            │
│                                                 ▼            │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────┐  │
│  │   SQLite    │◀───│    Diff     │◀───│  Compare with   │  │
│  │ State Store │    │   Engine    │    │  Previous State │  │
│  └─────────────┘    └─────────────┘    └─────────────────┘  │
│         │                                                    │
│         ▼                                                    │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────┐  │
│  │    Merge    │───▶│  Generate   │───▶│    Stage        │  │
│  │   Manifests │    │   Index     │    │   Output        │  │
│  └─────────────┘    └─────────────┘    └─────────────────┘  │
│                              │                               │
│                              ▼                               │
│                       ┌─────────────┐                        │
│                       │    MSIX     │                        │
│                       │  Packaging  │                        │
│                       └─────────────┘                        │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Component Responsibilities

| Component | Responsibility |
|-----------|----------------|
| **Scanner** | Discovers YAML manifests, reads file contents |
| **Hasher** | Computes SHA256 hashes for change detection |
| **Merger** | Combines multi-file manifests into canonical form |
| **Differ** | Compares current state against stored state |
| **State Manager** | Persists and retrieves state from SQLite |
| **Index Writer** | Generates the final source index (v1 or v2) |
| **Packager** | Creates MSIX files using platform tools |

## Build Pipeline

### Phase 1: Scanning

1. Walk the manifest repository directory tree
2. Identify YAML files (`.yaml`, `.yml`)
3. Group files by package identifier and version
4. Parse each file to extract metadata

### Phase 2: Hashing

For each file, compute:

- `raw_file_hash` — SHA256 of the raw file bytes (for caching)
- `version_content_sha256` — Hash of the canonical merged content
- `version_installer_sha256` — Hash of installer-affecting fields only

### Phase 3: Diffing

Compare current hashes against the state database:

- **Unchanged** — Skip reprocessing
- **Modified** — Rebuild that version
- **New** — Add to state
- **Deleted** — Remove from state (if not in repo)

### Phase 4: Merging

For changed versions:

1. Load all manifest files for the version
2. Merge fields following WinGet's precedence rules
3. Apply normalization (remove legal suffixes from names, etc.)
4. Validate the merged result

### Phase 5: Index Generation

Based on `--index-version`:

**v1 format:**

- Create `source.msix` with embedded SQLite index
- Generate hosted merged manifests

**v2 format:**

- Create `source2.msix` with optimized index
- Generate `packages/<id>/<hash>/versionData.mszyml` sidecars
- Generate hosted merged manifests

### Phase 6: Staging

Write all output to `state-dir/staging/build-<id>/`:

- This allows verification before committing
- Enables publishing the same build multiple times
- Supports rollback to previous builds

### Phase 7: Publishing

In the `publish` command:

1. Copy staged files to the output directory
2. Package `source.msix` or `source2.msix`
3. Sign the MSIX (if requested)
4. Update published state tracking

## State Management

The state store is a SQLite database that maintains:

### Tables

| Table | Purpose |
|-------|---------|
| `files` | Current file snapshots (path, hash, mtime) |
| `versions` | Version metadata and content hashes |
| `packages` | Package-level aggregated data |
| `builds` | Build history with timestamps and status |
| `published_files` | Inventory of files in published output |

### State Transitions

```
                    ┌─────────────┐
         ┌─────────▶│   Clean     │◀────────┐
         │          │   State     │         │
         │          └─────────────┘         │
         │                  │               │
    build│ with changes     │build          │clean
         │                  │no changes     │
         ▼                  │               │
┌─────────────────┐         │        ┌─────┴─────┐
│  Modified State │─────────┘        │ Unchanged │
│  (dirty files)  │                  │   State   │
└─────────────────┘                  └───────────┘
         │
         │publish
         ▼
┌─────────────────┐
│  Published      │
│  State          │
└─────────────────┘
```

### State Isolation

Key principle: The builder doesn't care about Git history. It compares:

- **Current repository state** — What's on disk right now
- **Last published state** — What's in the SQLite database

This means:

- You can switch branches, rewrite history, or amend commits
- The builder only sees the current file contents
- No dependency on Git being present or clean

## Hash Model

The builder uses multiple hash types for different purposes:

### Hash Types

```
┌─────────────────────────────────────────────────────────────┐
│                     Hash Hierarchy                           │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │            raw_file_hash                            │    │
│  │    (SHA256 of raw file bytes)                       │    │
│  │         Used for: Scan caching                      │    │
│  └─────────────────────────────────────────────────────┘    │
│                         │                                    │
│                         ▼                                    │
│  ┌─────────────────────────────────────────────────────┐    │
│  │       version_content_sha256                        │    │
│  │  (Hash of merged, canonical manifest)               │    │
│  │       Used for: Republish decisions                 │    │
│  └─────────────────────────────────────────────────────┘    │
│                         │                                    │
│                         ▼                                    │
│  ┌─────────────────────────────────────────────────────┐    │
│  │      version_installer_sha256                       │    │
│  │ (Hash of installer URLs, types, architectures)      │    │
│  │      Used for: Validation routing                   │    │
│  └─────────────────────────────────────────────────────┘    │
│                         │                                    │
│                         ▼                                    │
│  ┌─────────────────────────────────────────────────────┐    │
│  │     published_manifest_sha256                       │    │
│  │    (Hash of final hosted manifest bytes)            │    │
│  │         Used for: Integrity verification            │    │
│  └─────────────────────────────────────────────────────┘    │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Content Exclusions

Certain fields are excluded from specific hashes:

| Field | Content Hash | Installer Hash |
|-------|--------------|----------------|
| `PackageIdentifier` | ✓ | ✓ |
| `PackageVersion` | ✓ | ✓ |
| `InstallerUrl` | ✓ | ✓ |
| `InstallerSha256` | ✓ | ✓ |
| `Commands` | ✓ | ✗ |
| `Protocols` | ✓ | ✗ |
| `FileExtensions` | ✓ | ✗ |

This means changing `Commands` requires republishing (content hash change) but not revalidation (installer hash unchanged).

### Hash Examples

**Scenario 1: Fix a typo in the package description**

- `raw_file_hash` — Changes
- `version_content_sha256` — Changes
- `version_installer_sha256` — Unchanged
- **Result:** Republish the manifest, no validation needed

**Scenario 2: Update an installer URL**

- `raw_file_hash` — Changes
- `version_content_sha256` — Changes
- `version_installer_sha256` — Changes
- **Result:** Republish and add to validation queue

**Scenario 3: No changes to a file**

- All hashes match stored state
- **Result:** Skip reprocessing entirely

## Output Formats

### v1 Format (Legacy)

```
output/
├── source.msix              # Main index package
└── manifests/
    └── v/
        └── Vendor/
            └── App/
                └── <hash>/
                    └── manifest.yaml    # Hosted merged manifest
```

**Characteristics:**

- Single `source.msix` with embedded SQLite
- All version data in the index
- Larger initial download for clients

### v2 Format (Recommended)

```
output/
├── source2.msix             # Main index package (smaller)
├── packages/
│   └── Vendor.App/
│       └── <hash8>/
│           └── versionData.mszyml    # Per-version metadata
└── manifests/
    └── v/
        └── Vendor/
            └── App/
                └── <hash>/
                    └── manifest.yaml    # Hosted merged manifest
```

**Characteristics:**

- Smaller initial `source2.msix` download
- Lazy-loaded version data via `versionData.mszyml`
- Better for repositories with many packages
- Requires WinGet client 1.5+

### Format Selection Guidance

| Situation | Recommended Format |
|-----------|-------------------|
| New repository | v2 |
| Existing v1 users | v2 (with migration guide) |
| Legacy WinGet clients (< 1.5) | v1 |
| Large repository (1000+ packages) | v2 |

## Backend Abstraction

The builder supports two backends for index operations:

### Rust Backend (Default)

- Pure Rust implementation
- Cross-platform (Windows, Linux, macOS)
- Direct SQLite index writing
- Recommended for all use cases

### WinGetUtil Backend

- Uses Microsoft's `WinGetUtil.dll`
- Windows only
- Maximum compatibility with official WinGet
- Requires `WinGetUtil.dll` at runtime

### Backend Selection

```powershell
# Default (Rust)
winget-source-builder build --backend rust

# Windows with official compatibility
winget-source-builder build --backend wingetutil
```

The Rust backend is recommended unless you specifically need `WinGetUtil.dll` compatibility for validation purposes.
