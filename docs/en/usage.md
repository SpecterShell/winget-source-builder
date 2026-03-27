# Usage Guide

This guide walks you through using `winget-source-builder` to create and maintain a WinGet-compatible package source. By the end, you'll understand the two-phase workflow (build → publish) and how to take advantage of incremental builds.

## Table of Contents

- [Overview](#overview)
- [Prerequisites](#prerequisites)
- [Your First Build](#your-first-build)
- [Understanding Incremental Builds](#understanding-incremental-builds)
- [Common Tasks](#common-tasks)
- [Environment Variables](#environment-variables)
- [Troubleshooting](#troubleshooting)

## Overview

`winget-source-builder` operates in two distinct phases:

1. **Build phase** — Scans your manifest repository, computes hashes, identifies changes, and prepares a staging tree
2. **Publish phase** — Packages the staging tree into `source.msix` (v1) or `source2.msix` (v2) and writes the final output

This separation lets you verify the build before publishing, and it enables incremental workflows where you might build frequently but publish only when ready.

## Prerequisites

Before you begin, you'll need:

- **A WinGet-style manifest repository** — YAML manifests organized by package identifier and version (e.g., `manifests/v/Vendor/App/1.0.0/`)
- **A writable state directory** — Where the builder tracks incremental state, staging builds, and validation queues
- **Packaging assets** (for publish) — An `AppxManifest.xml` and `Assets/` directory for MSIX packaging
- **Platform requirements:**
  - Windows: `WinGetUtil.dll` (bundled with releases)
  - Linux/macOS: `makemsix` for MSIX packaging

A template repository with sample manifests and packaging assets is available at [winget-source-template](https://github.com/SpecterShell/winget-source-template).

## Your First Build

### Step 1: Initial Setup

Create your state directory. This is where all builder state lives:

```powershell
mkdir ./state
```

The builder will create several files here:

- `state.sqlite` — The main state database
- `validation-queue.json` — Pending installer validations
- `staging/` — Staged builds ready for publishing

### Step 2: Run the Build Command

The `build` command analyzes your manifest repository and prepares everything for publishing:

```powershell
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --backend rust `
  --index-version v2
```

Here's what happens:

1. **Scanning** — All YAML manifests are discovered and read
2. **Hashing** — File contents are hashed for change detection
3. **Merging** — Multi-file manifests are merged into canonical form
4. **Diffing** — Changes are compared against the previous state
5. **Staging** — A publishable tree is created in `state/staging/`

The `--index-version v2` flag produces the modern source format (`source2.msix`). Use `v1` only if you need backward compatibility with older WinGet clients.

### Step 3: Check the Results

After building, check the status:

```powershell
winget-source-builder status --state-dir ./state
```

This shows:

- How many packages and versions are in your working state
- The latest staged build (ready to publish)
- Any pending changes detected in the repository

### Step 4: Publish

Now create the final MSIX package:

```powershell
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging
```

The output directory now contains:

- `source2.msix` — The signed (or unsigned) MSIX package
- `manifests/` — Hosted merged manifests for direct download
- `packages/` — Version data sidecars (v2 format only)

Deploy these files to your web server, and users can add your source:

```powershell
winget source add --name mysource --argument https://your-domain.com/source2.msix
```

## Understanding Incremental Builds

One of the key benefits of this tool is incremental building. Here's how it works:

### First Build vs Subsequent Builds

**First build:**

- Scans all manifests
- Computes hashes for every file
- Processes every version
- Takes longer (minutes for large repositories)

**Subsequent builds:**

- Compares file hashes against stored state
- Only processes changed versions
- Typically completes in seconds

### What Triggers a Full Rebuild

Certain changes require reprocessing more than just the changed file:

| Change Type | What Gets Reprocessed |
|-------------|----------------------|
| Single manifest edit | Just that version |
| Installer URL change | Version + validation queue entry |
| Package name change | All versions (package-level metadata) |
| Schema changes | May require `--force` |

### Forcing a Clean Build

If you suspect state corruption or want to start fresh:

```powershell
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --index-version v2 `
  --force
```

The `--force` flag ignores existing state and rebuilds everything.

## Common Tasks

### Adding a New Package Version

1. Add your manifest files to the repository (e.g., `manifests/v/Vendor/App/1.2.3/`)
2. Run build to pick up the changes
3. Publish when ready

For quick testing of a single version without scanning the entire repo:

```powershell
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --version-dir ./manifests/v/Vendor/App/1.2.3
```

### Removing a Package

To remove a specific version:

```powershell
winget-source-builder remove `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.0.0
```

To remove an entire package (all versions), repeat for each version or use `clean` to reset state.

### Checking Repository Status

Get a quick overview:

```powershell
winget-source-builder status --state-dir ./state
```

See detailed pending changes:

```powershell
winget-source-builder diff `
  --repo-dir ./manifests `
  --state-dir ./state
```

For machine-readable output (useful in CI):

```powershell
winget-source-builder diff --json > changes.json
```

### Validating Before Publish

Verify the staged build is correct:

```powershell
winget-source-builder verify staged --state-dir ./state
```

Verify a published output:

```powershell
winget-source-builder verify published `
  --state-dir ./state `
  --out-dir ./publish
```

### Code Signing

**On Windows** (using `signtool.exe`):

```powershell
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./signing.pfx `
  --sign-password-env WINGET_SOURCE_SIGN_PASSWORD `
  --timestamp-url http://timestamp.digicert.com
```

**On Linux/macOS** (using `makemsix` + `openssl`):

```powershell
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./signing.pfx `
  --sign-password-env WINGET_SOURCE_SIGN_PASSWORD
```

Note: Timestamp URLs are currently only supported on Windows.

### Viewing Package Information

Inspect a package in the state database:

```powershell
# Show package details
winget-source-builder show package --state-dir ./state Vendor.App

# Show specific version
winget-source-builder show version `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.2.3

# Output as JSON for scripting
winget-source-builder show package Vendor.App --json
```

### Cleaning Up

Over time, the state directory grows. Clean up old builds:

```powershell
# Keep only the last 5 builds
winget-source-builder clean `
  --state-dir ./state `
  --builds `
  --keep-last 5

# Remove old staging directories
winget-source-builder clean `
  --state-dir ./state `
  --staging

# Nuclear option: clean everything except working state
winget-source-builder clean `
  --state-dir ./state `
  --all
```

### Diagnosing Issues

Run the diagnostic command to check your environment:

```powershell
winget-source-builder doctor `
  --repo-dir ./manifests `
  --state-dir ./state `
  --packaging-assets-dir ./packaging
```

This checks:

- Required tools are available
- Packaging assets are valid
- Backend compatibility
- State database health

## Environment Variables

The following environment variables modify behavior. Use them when the defaults don't fit your setup.

| Variable | Purpose |
|----------|---------|
| `WINGET_CLI_ROOT` | Path to `winget-cli` checkout for building with custom `WinGetUtil.dll` |
| `MSIX_PACKAGING_ROOT` | Path to `msix-packaging` checkout for custom `makemsix` (use Mozilla's fork for signing support) |
| `MAKEAPPX_EXE` | Explicit path to `makeappx.exe` (Windows) |
| `MAKEMSIX_EXE` | Explicit path to `makemsix` (Linux/macOS) |
| `OPENSSL` | Explicit path to `openssl` binary |
| `WINGET_SOURCE_BUILDER_WORKSPACE_ROOT` | Fallback workspace root for packaging asset auto-discovery |
| `WINGET_SOURCE_BUILDER_LANG` | Runtime locale for CLI messages (e.g., `en`, `zh-CN`) |

## Troubleshooting

### Build fails with "backend unavailable"

**Problem:** The `wingetutil` backend requires Windows and `WinGetUtil.dll`.

**Solution:** Use `--backend rust` on non-Windows platforms, or ensure `WinGetUtil.dll` is next to the executable.

### Publish fails with "output directory drift"

**Problem:** The output directory contains files from a different build.

**Solution:** Use `--force` to overwrite, or clean the output directory first.

### Incremental build not detecting changes

**Problem:** File timestamps changed but content didn't (or vice versa).

**Solution:** The builder uses SHA256 content hashes, not timestamps. If you're not seeing expected changes, verify the files actually differ.

### Staged build lost after build

**Problem:** Something deleted or corrupted the staging directory.

**Solution:** Run `build --force` to recreate the staging tree.

### MSIX signing fails on Linux/macOS

**Problem:** Default `makemsix` doesn't support signing.

**Solution:** Set `MSIX_PACKAGING_ROOT` to Mozilla's signing-capable fork of `msix-packaging` and ensure `openssl` is installed.

### Manifest merge errors

**Problem:** Multi-file manifests have conflicting fields.

**Solution:** Use the `merge` command to debug:

```powershell
winget-source-builder merge `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3
```

This shows the merged output without affecting state.
