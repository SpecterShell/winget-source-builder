# Development Guide

This guide covers building `winget-source-builder` from source, running tests, and contributing to the project.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Building from Source](#building-from-source)
- [Project Structure](#project-structure)
- [Running Tests](#running-tests)
- [Localization](#localization)
- [Contributing](#contributing)

## Prerequisites

### Required Tools

- **Rust** — Latest stable version (1.70+ recommended)
  - Install via [rustup](https://rustup.rs/)
- **Git** — For cloning and submodule management

### Platform-Specific Requirements

**Windows:**

- PowerShell 7+ (for running build scripts)
- Visual Studio 2022 Build Tools or full Visual Studio
  - Please refer to [the winget-cli's guide](https://github.com/microsoft/winget-cli/blob/master/doc/Developing.md) on how to configure Visual Studio.
- Windows SDK

**Linux:**

- GCC or Clang toolchain
- CMake (3.15+)
- OpenSSL development headers (for signing support)

**macOS:**

- Xcode Command Line Tools
- CMake (3.15+)
- OpenSSL (via Homebrew: `brew install openssl`)

### Clone the Repository

```powershell
git clone https://github.com/SpecterShell/winget-source-builder.git
cd winget-source-builder

# Initialize submodules (required for WinGetUtil and makemsix)
git -c submodule.recurse=false submodule update --init winget-cli msix-packaging
```

The submodules provide:

- `winget-cli/` — Source for building `WinGetUtil.dll` (Windows)
- `msix-packaging/` — Source for building `makemsix` (Linux/macOS)

## Building from Source

### Standard Build

```powershell
# Debug build (faster compilation, slower execution)
cargo build

# Release build (slower compilation, optimized execution)
cargo build --release
```

The first build will:

1. Compile the Rust code
2. On Windows: Build `WinGetUtil.dll` from the `winget-cli` submodule
3. On Linux/macOS: Build `makemsix` from the `msix-packaging` submodule

### Build Outputs

**Windows:**

- `target/debug/winget-source-builder.exe`
- `target/debug/WinGetUtil.dll` (copied from build)

**Linux/macOS:**

- `target/debug/winget-source-builder`
- `target/debug/makemsix` (built from submodule)

### Custom WinGetUtil Location

If you have a separate `winget-cli` checkout:

```powershell
$env:WINGET_CLI_ROOT = "C:\path\to\winget-cli"
cargo build --release
```

### Custom makemsix Location

For Linux/macOS, if you have a separate `msix-packaging` checkout:

```powershell
$env:MSIX_PACKAGING_ROOT = "/path/to/msix-packaging"
cargo build --release
```

### Using Mozilla's Signing-Capable makemsix

The default `msix-packaging` doesn't support signing on non-Windows platforms. For signing support on Linux/macOS:

```powershell
git clone https://github.com/mozilla/msix-packaging.git $env:MSIX_PACKAGING_ROOT
cargo build --release
```

## Project Structure

```
winget-source-builder/
├── src/
│   ├── main.rs           # CLI entry point, command routing
│   ├── adapter.rs        # Backend abstraction layer
│   ├── backend.rs        # Backend implementations
│   ├── builder.rs        # Core build orchestration
│   ├── i18n.rs           # Internationalization setup
│   ├── manifest.rs       # Manifest parsing and merging
│   ├── mszip.rs          # ZIP compression utilities
│   ├── progress.rs       # Progress reporting
│   ├── state.rs          # State database operations
│   └── version.rs        # Version comparison and normalization
├── locales/              # Translation files
│   ├── en.yml
│   ├── zh-CN.yml
│   └── zh-TW.yml
├── scripts/              # Build helper scripts
│   ├── build-wingetutil.ps1
│   └── build-makemsix.sh
├── docs/                 # Documentation
│   └── en/
│       ├── usage.md
│       ├── cli-reference.md
│       ├── architecture.md
│       └── development.md
├── winget-cli/           # Git submodule (Windows backend)
├── msix-packaging/       # Git submodule (cross-platform packaging)
└── Cargo.toml
```

### Key Modules

| Module | Purpose |
|--------|---------|
| `builder.rs` | Orchestrates the build pipeline: scan, hash, diff, merge, index |
| `state.rs` | SQLite database operations for incremental state |
| `manifest.rs` | YAML parsing, multi-file merging, canonicalization |
| `adapter.rs` | Abstraction over `wingetutil` and `rust` backends |
| `backend.rs` | Backend implementations for index operations |
| `progress.rs` | Progress reporting during long operations |

## Running Tests

### Quick Test Check

Before submitting changes, run these commands:

```powershell
# Format check
cargo fmt --all --check

# Linting
cargo clippy --all-targets --all-features -- -D warnings

# Run all tests
cargo test --verbose
```

### Test Categories

**Unit Tests:**

```powershell
# Run only unit tests
cargo test --lib --verbose
```

Unit tests cover:

- Manifest merging logic
- Hash computation and filtering
- Version comparison
- State database operations

**End-to-End Tests:**

```powershell
# Run all tests including e2e (requires platform dependencies)
cargo test --verbose
```

E2E tests:

- Build the fixture repository in `tests/data/e2e-repo/`
- Test the full build → publish pipeline
- Verify output integrity
- Skip automatically if platform dependencies are missing

**Platform-Specific Notes:**

- **Windows:** E2E tests run with both `wingetutil` and `rust` backends
- **Linux/macOS:** E2E tests run with `rust` backend only (requires `makemsix`)

### Test Fixtures

The `tests/data/e2e-repo/` directory contains a sample manifest repository used by E2E tests. It includes:

- Multi-file manifests
- Various installer types
- Edge cases for merging and validation

When adding features, consider adding test cases to this fixture.

## Localization

The project uses `rust-i18n` for internationalization. Translations are stored in YAML files under `locales/`.

### Adding a New Language

1. Create a new file: `locales/<locale>.yml`
2. Copy the structure from `locales/en.yml`
3. Translate all values
4. Test by running with `WINGET_SOURCE_BUILDER_LANG=<locale>`

### Translation File Structure

```yaml
# locales/en.yml
hello: Hello
build:
  scanning: Scanning repository...
  complete: Build complete
error:
  not_found: File not found
```

### Using Translations in Code

```rust
use rust_i18n::t;

println!("{}", t!("build.scanning"));
```

### Testing Localization

```powershell
# Test English (default)
winget-source-builder --lang en status --state-dir ./state

# Test Simplified Chinese
winget-source-builder --lang zh-CN status --state-dir ./state

# Or via environment variable
$env:WINGET_SOURCE_BUILDER_LANG = "zh-TW"
winget-source-builder status --state-dir ./state
```

## Contributing

### Getting Started

1. Fork the repository on GitHub
2. Clone your fork locally
3. Create a new branch for your feature or fix
4. Make your changes
5. Run the test checklist (format, clippy, tests)
6. Commit with a clear message
7. Push and open a Pull Request

### Code Style

- Follow `rustfmt` conventions (`cargo fmt`)
- Address all `clippy` warnings
- Write doc comments for public APIs
- Add tests for new functionality

### Commit Messages

Use clear, descriptive commit messages:

```
Add support for custom validation queues

- Add --validation-queue-dir option
- Update state tracking for queue files
- Add tests for queue persistence
```

### Pull Request Process

1. Ensure all CI checks pass
2. Update documentation if needed
3. Link any related issues
4. Request review from maintainers

### Reporting Issues

When reporting bugs, include:

- Operating system and version
- Rust version (`rustc --version`)
- Builder version or commit hash
- Steps to reproduce
- Expected vs actual behavior
- Full error output (with `--verbose` if available)

### CI/CD

The project uses GitHub Actions for CI:

- **ci.yml** — Runs on every push and PR
  - Format checking
  - Clippy linting
  - Tests on Linux, macOS, Windows
  - Build artifacts for each platform

- **release.yml** — Runs on version tags (`v*`)
  - Release builds
  - Asset packaging
  - GitHub Release creation

See [Contributing Guide](contributing.md) for more details on CI/CD and release workflows.
