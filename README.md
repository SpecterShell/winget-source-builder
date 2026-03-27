# winget-source-builder

[简体中文](README.zh-CN.md) | [繁體中文](README.zh-TW.md)

[![CI](https://github.com/SpecterShell/winget-source-builder/actions/workflows/ci.yml/badge.svg)](https://github.com/SpecterShell/winget-source-builder/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/SpecterShell/winget-source-builder)](https://github.com/SpecterShell/winget-source-builder/releases)

> Build WinGet-compatible source indexes from your manifest repository — fast, incremental, and ready to publish.

`winget-source-builder` helps you run your own WinGet package repository. If you maintain a collection of software manifests and want users to install from your source using `winget source add`, this tool builds the required indexes and packages.

**The problem it solves:** Creating a valid WinGet source involves complex indexing, hashing, and MSIX packaging. Doing this manually is error-prone and slow. This tool automates the entire pipeline.

**How it works:** The builder scans your YAML manifests, tracks file changes using SHA256 hashes, and maintains an incremental state database. On each run, it only processes what changed — making subsequent builds nearly instantaneous. It outputs a ready-to-deploy MSIX package (`source.msix` or `source2.msix`) plus hosted manifest files.

**Who it's for:** Package repository maintainers, software distributors, and organizations that need a private or public WinGet source alternative to the Microsoft Community Repository.

## Installation

### Download Pre-built Binary

Grab the latest release for your platform from the [GitHub releases page](https://github.com/SpecterShell/winget-source-builder/releases).

**Windows:** Extract the zip and place `winget-source-builder.exe` and `WinGetUtil.dll` in your PATH or use directly.

**Linux/macOS:** Extract the archive. You'll need `makemsix` available for packaging (see [Development Guide](docs/en/development.md) for building it).

### Build from Source

```powershell
git clone https://github.com/SpecterShell/winget-source-builder.git
cd winget-source-builder
git -c submodule.recurse=false submodule update --init winget-cli msix-packaging
cargo build --release
```

See the [Development Guide](docs/en/development.md) for detailed setup instructions.

## Quick Start

The workflow has two phases: **build** (prepare) and **publish** (package).

```powershell
# Step 1: Build the source index
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --index-version v2

# Step 2: Publish the MSIX package
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging
```

After running these commands, you'll find:

- `publish/source2.msix` — The main source package (for index v2)
- `publish/manifests/` — Hosted merged manifests
- `publish/packages/` — Version data sidecars (v2 only)

Users can then add your source:

```powershell
winget source add --name mysource --argument https://your-domain.com/source2.msix
```

## Common Workflows

### First-Time Setup

New to WinGet sources? Start with the [Usage Guide](docs/en/usage.md) for a complete walkthrough including:

- Setting up your manifest repository structure
- Creating packaging assets (AppxManifest.xml, icons)
- Running your first build

### Daily Operations (Adding/Updating Packages)

When you add or update manifests in your repository:

```powershell
# Just run build — it will detect changes and update incrementally
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --index-version v2

# Check what changed
winget-source-builder diff --repo-dir ./manifests --state-dir ./state
```

The builder compares file hashes against the previous state. Only changed versions are reprocessed.

### Publishing a Release

When you're ready to publish:

```powershell
# Basic publish (unsigned)
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging

# Or with code signing (Windows)
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./cert.pfx `
  --sign-password-env CERT_PASSWORD
```

### Checking What's Changed

Before building, see what differs from your last published state:

```powershell
winget-source-builder diff --repo-dir ./manifests --state-dir ./state
```

Or get a full status report:

```powershell
winget-source-builder status --repo-dir ./manifests --state-dir ./state
```

## Next Steps

- **[Usage Guide](docs/en/usage.md)** — Step-by-step tutorials covering your first build, understanding incremental builds, and common tasks
- **[CLI Reference](docs/en/cli-reference.md)** — Complete command documentation with examples
- **[Architecture](docs/en/architecture.md)** — How it works under the hood: hash model, state management, and build pipeline
- **[Development](docs/en/development.md)** — Building from source, running tests, and contributing
- **[Contributing](docs/en/contributing.md)** — CI/CD workflows and release process

## License

MIT License — see [LICENSE](LICENSE) for details.
