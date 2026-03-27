# Contributing Guide

This document covers the CI/CD workflows, release process, and guidelines for downstream projects consuming this tool.

## Table of Contents

- [CI/CD Workflows](#cicd-workflows)
- [Release Process](#release-process)
- [Release Package Contents](#release-package-contents)
- [For Downstream Projects](#for-downstream-projects)
- [Versioning](#versioning)

## CI/CD Workflows

The project uses GitHub Actions for continuous integration and deployment.

### CI Workflow (`ci.yml`)

Triggered on every push and pull request to `main`.

**What it does:**

1. **Code Quality Checks**
   - `cargo fmt --all --check` — Ensures consistent formatting
   - `cargo clippy --all-targets --all-features -- -D warnings` — Linting

2. **Testing**
   - Runs `cargo test --verbose` on:
     - Ubuntu (latest)
     - macOS (latest)
     - Windows (latest)
   - Initializes submodules for WinGetUtil and makemsix
   - Tests automatically skip when platform dependencies are unavailable

3. **Build Artifacts**
   - Produces release binaries for:
     - Linux x86_64
     - macOS (universal)
     - Windows x86_64
     - Windows aarch64
   - Uploads as workflow artifacts (retained for 90 days)

**Viewing results:**

- Go to the [Actions tab](https://github.com/SpecterShell/winget-source-builder/actions)
- Click on a workflow run
- Download artifacts from the summary page

### Release Workflow (`release.yml`)

Triggered when a tag matching `v*` is pushed (e.g., `v1.2.3`).

**What it does:**

1. **Build Phase**
   - Checks out code and submodules
   - Builds release binaries for all platforms
   - Provisions WinGetUtil.dll (Windows) and makemsix (Linux/macOS)

2. **Packaging Phase**
   - Creates platform-specific archives:
     - Windows: `.zip` files
     - Linux/macOS: `.tar.gz` files
   - Includes appropriate helper binaries

3. **Release Phase**
   - Creates or updates a GitHub Release
   - Uploads all platform packages
   - Generates release notes from commits

**Creating a release:**

```powershell
# Tag the release
git tag v1.2.3

# Push the tag (triggers release workflow)
git push origin v1.2.3
```

## Release Process

### Version Numbering

This project follows [Semantic Versioning](https://semver.org/):

- **MAJOR** — Breaking changes (CLI changes, output format changes)
- **MINOR** — New features, backwards compatible
- **PATCH** — Bug fixes, backwards compatible

### Release Checklist

Before creating a release:

- [ ] Update `CHANGELOG.md` with release notes
- [ ] Ensure all tests pass on `main`
- [ ] Update version in `Cargo.toml` if not already done
- [ ] Verify documentation is up to date
- [ ] Create and push the version tag

### Pre-Releases

For beta or testing versions:

```powershell
# Create a pre-release version
git tag v1.3.0-beta.1
git push origin v1.3.0-beta.1
```

GitHub Releases will mark these as pre-releases automatically.

## Release Package Contents

### Windows Package

```
winget-source-builder-x86_64-pc-windows-msvc.zip
├── winget-source-builder.exe    # Main executable
├── WinGetUtil.dll               # Windows backend library
└── LICENSE                      # License file
```

### Linux Package

```
winget-source-builder-x86_64-unknown-linux-gnu.tar.gz
├── winget-source-builder        # Main executable
├── makemsix                     # MSIX packaging tool
└── LICENSE                      # License file
```

### macOS Package

```
winget-source-builder-universal-apple-darwin.tar.gz
├── winget-source-builder        # Main executable (universal binary)
├── makemsix                     # MSIX packaging tool
└── LICENSE                      # License file
```

### Notes

- **Windows:** `WinGetUtil.dll` must be in the same directory as the executable
- **Linux/macOS:** `makemsix` must be in the same directory or in PATH
- Packaged runtime builds expect helper binaries side by side

## For Downstream Projects

If you're using `winget-source-builder` in your own CI/CD pipeline, here's how to integrate it effectively.

### Downloading Releases

**Recommended: Use a release downloader action**

```yaml
# GitHub Actions example
- name: Download winget-source-builder
  uses: robinraju/release-downloader@v1
  with:
    repository: SpecterShell/winget-source-builder
    tag: v1.0.0  # Pin to a specific version
    fileName: winget-source-builder-x86_64-pc-windows-msvc.zip
    extract: true
```

**PowerShell example:**

```powershell
$version = "1.0.0"
$url = "https://github.com/SpecterShell/winget-source-builder/releases/download/v$version/winget-source-builder-x86_64-pc-windows-msvc.zip"

Invoke-WebRequest -Uri $url -OutFile builder.zip
Expand-Archive -Path builder.zip -DestinationPath ./builder
```

### Pinning Versions

For reproducible builds, always pin to a specific version:

```yaml
# Good: Pinned version
- uses: robinraju/release-downloader@v1
  with:
    tag: v1.2.3

# Avoid: Latest release (may break unexpectedly)
- uses: robinraju/release-downloader@v1
  with:
    latest: true
```

### Caching

To speed up CI, cache the downloaded binary:

```yaml
- name: Cache winget-source-builder
  uses: actions/cache@v3
  with:
    path: ./builder
    key: builder-${{ runner.os }}-${{ env.BUILDER_VERSION }}

- name: Download if not cached
  if: steps.cache.outputs.cache-hit != 'true'
  uses: robinraju/release-downloader@v1
  with:
    tag: ${{ env.BUILDER_VERSION }}
    fileName: winget-source-builder-x86_64-pc-windows-msvc.zip
    extract: true
```

### Example CI Integration

```yaml
name: Build Source

on:
  push:
    paths:
      - 'manifests/**'

env:
  BUILDER_VERSION: '1.0.0'

jobs:
  build:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4

      - name: Download builder
        uses: robinraju/release-downloader@v1
        with:
          repository: SpecterShell/winget-source-builder
          tag: v${{ env.BUILDER_VERSION }}
          fileName: winget-source-builder-x86_64-pc-windows-msvc.zip
          extract: true

      - name: Build source
        run: |
          ./winget-source-builder.exe build `
            --repo-dir ./manifests `
            --state-dir ./state `
            --index-version v2

      - name: Publish source
        run: |
          ./winget-source-builder.exe publish `
            --state-dir ./state `
            --out-dir ./publish `
            --packaging-assets-dir ./packaging

      - name: Upload artifacts
        uses: actions/upload-artifact@v3
        with:
          name: source
          path: ./publish/
```

### State Persistence

For incremental builds across CI runs, persist the state directory:

```yaml
- name: Restore state cache
  uses: actions/cache@v3
  with:
    path: ./state
    key: builder-state-${{ github.run_id }}
    restore-keys: |
      builder-state-

- name: Build (incremental)
  run: |
    ./winget-source-builder.exe build `
      --repo-dir ./manifests `
      --state-dir ./state `
      --index-version v2
```

## Versioning

### Compatibility Guarantee

Within a major version:

- CLI arguments remain backwards compatible
- Output formats remain stable
- State database migrations are automatic

Breaking changes (major version bumps) will be documented in the release notes with migration guides.

### Deprecation Policy

Before removing features:

1. Feature is marked deprecated in documentation
2. Warnings are added to CLI output
3. At least one minor version with warnings
4. Removal in next major version

### API Stability

The tool is a CLI application, not a library. The public interface is:

- Command-line arguments
- Exit codes
- JSON output format (when using `--json`)
- Environment variables

Internal APIs may change without notice.
