# CLI Reference

Complete reference for all `winget-source-builder` commands, options, and exit codes.

## Table of Contents

- [Global Options](#global-options)
- [Command Groups](#command-groups)
  - [Core Workflow](#core-workflow)
  - [Repository Management](#repository-management)
  - [Inspection & Debugging](#inspection--debugging)
  - [Maintenance](#maintenance)
- [Exit Codes](#exit-codes)

## Global Options

These options work with most commands:

| Option | Description |
|--------|-------------|
| `--lang <locale>` | Override the display language (e.g., `en`, `zh-CN`, `zh-TW`) |
| `--dry-run` | Show what would happen without making changes |
| `--force` | Overwrite existing data, ignore safety checks |
| `--json` | Output machine-readable JSON (on reporting commands) |

### Index Version Selection

Many commands accept `--index-version` to choose the source format:

- `--index-version v1` — Legacy format (`source.msix`)
- `--index-version v2` — Modern format (`source2.msix`, recommended)

### Display Version Conflict Strategy

For commands that mutate state, you can control how ARP version conflicts are handled:

| Strategy | Behavior |
|----------|----------|
| `latest` | Keep the latest version (default) |
| `oldest` | Keep the oldest version |
| `strip-all` | Remove all conflicting display versions |
| `error` | Fail if conflicts are detected |

## Command Groups

### Core Workflow

Commands you'll use daily: `build` and `publish`.

#### `build`

**Purpose:** Scan the repository, identify changes, update state, and stage a publishable build.

**When to use it:** Run this after adding, updating, or removing manifests. It's the first step in the build → publish workflow.

```powershell
winget-source-builder build `
  --repo-dir <dir> `
  --state-dir <dir> `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--package-id <id>...] `
  [--version-dir <dir>...] `
  [--force] `
  [--dry-run] `
  [--no-validation-queue] `
  [--display-version-conflict-strategy <latest|oldest|strip-all|error>]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--repo-dir` | **Required.** Path to the manifest repository |
| `--state-dir` | **Required.** Path to the state directory |
| `--backend` | Backend for index operations: `wingetutil` (Windows only) or `rust` (default) |
| `--index-version` | Source format version: `v1` or `v2` (default: `v2`) |
| `--package-id` | Restrict build to specific package(s), repeatable |
| `--version-dir` | Restrict build to specific version directory(s), repeatable |
| `--force` | Ignore existing state and rebuild everything |
| `--dry-run` | Show what would change without updating state |
| `--no-validation-queue` | Skip generating validation-queue.json |
| `--display-version-conflict-strategy` | How to handle ARP version conflicts |

**Examples:**

```powershell
# Full repository build
winget-source-builder build --repo-dir ./manifests --state-dir ./state

# Incremental build for v2 format with rust backend
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --backend rust `
  --index-version v2

# Build only specific packages
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App1 `
  --package-id Vendor.App2

# Force rebuild everything
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --force
```

#### `publish`

**Purpose:** Package the staged build into an MSIX and write the final publish tree.

**When to use it:** Run this after `build` when you're ready to deploy. This creates the distributable files.

```powershell
winget-source-builder publish `
  --state-dir <dir> `
  --out-dir <dir> `
  --packaging-assets-dir <dir> `
  [--build-id <id>] `
  [--force] `
  [--dry-run] `
  [--sign-pfx-file <file>] `
  [--sign-password <value>] `
  [--sign-password-env <ENV>] `
  [--timestamp-url <url>]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--state-dir` | **Required.** Path to the state directory |
| `--out-dir` | **Required.** Path for the final output |
| `--packaging-assets-dir` | **Required.** Directory containing `AppxManifest.xml` and `Assets/` |
| `--build-id` | Publish a specific build instead of the latest staged |
| `--force` | Overwrite output directory even if it differs from tracked state |
| `--dry-run` | Show what would be written without creating files |
| `--sign-pfx-file` | Path to PFX certificate for code signing |
| `--sign-password` | Password for the PFX file (use `--sign-password-env` instead for security) |
| `--sign-password-env` | Environment variable containing the PFX password |
| `--timestamp-url` | Timestamp server URL (Windows only) |

**Examples:**

```powershell
# Basic publish (unsigned)
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging

# Publish with code signing
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./cert.pfx `
  --sign-password-env CERT_PASSWORD

# Publish a specific previous build
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --build-id 42
```

---

### Repository Management

Commands for manipulating state without full rebuilds.

#### `add`

**Purpose:** Incrementally add a specific version to the working state.

**When to use it:** When you want to add a single version without scanning the entire repository. Faster than `build` for targeted updates.

```powershell
winget-source-builder add `
  --repo-dir <dir> `
  --state-dir <dir> `
  (--version-dir <dir>... | --manifest-file <file>... | --package-id <id> --version <ver>) `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--force] `
  [--dry-run] `
  [--no-validation-queue] `
  [--display-version-conflict-strategy <latest|oldest|strip-all|error>]
```

**Examples:**

```powershell
# Add by version directory
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --version-dir ./manifests/v/Vendor/App/1.2.3

# Add by package ID and version
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.2.3

# Add a single manifest file
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --manifest-file ./manifests/v/Vendor/App/1.2.3/Vendor.App.yaml
```

#### `remove` / `delete`

**Purpose:** Incrementally remove a specific version from the working state.

**When to use it:** When you need to remove a version without rebuilding everything. `delete` is an exact alias for `remove`.

```powershell
winget-source-builder remove `
  --repo-dir <dir> `
  --state-dir <dir> `
  (--version-dir <dir>... | --manifest-file <file>... | --package-id <id> --version <ver>) `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--force] `
  [--dry-run] `
  [--no-validation-queue] `
  [--display-version-conflict-strategy <latest|oldest|strip-all|error>]
```

**Examples:**

```powershell
# Remove by package ID and version
winget-source-builder remove `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.0.0

# Remove by version directory
winget-source-builder remove `
  --repo-dir ./manifests `
  --state-dir ./state `
  --version-dir ./manifests/v/Vendor/App/1.0.0
```

#### `diff`

**Purpose:** Compare the current repository contents against the working state.

**When to use it:** To see what has changed before running `build`. Useful in CI to decide if a build is needed.

```powershell
winget-source-builder diff `
  --repo-dir <dir> `
  --state-dir <dir> `
  [--package-id <id>...] `
  [--version-dir <dir>...] `
  [--json]
```

**Examples:**

```powershell
# Human-readable diff
winget-source-builder diff --repo-dir ./manifests --state-dir ./state

# Machine-readable diff for CI
winget-source-builder diff `
  --repo-dir ./manifests `
  --state-dir ./state `
  --json > changes.json

# Diff only specific packages
winget-source-builder diff `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App
```

#### `status`

**Purpose:** Show the current state summary, build pointers, and optional diff information.

**When to use it:** To get a quick overview of the repository state without a full diff.

```powershell
winget-source-builder status `
  --state-dir <dir> `
  [--repo-dir <dir>] `
  [--json]
```

**Examples:**

```powershell
# Quick status overview
winget-source-builder status --state-dir ./state

# Include pending changes in status
winget-source-builder status `
  --state-dir ./state `
  --repo-dir ./manifests

# JSON output for scripting
winget-source-builder status --state-dir ./state --json
```

---

### Inspection & Debugging

Commands for looking at data and verifying consistency.

#### `list-builds`

**Purpose:** Display recent build records from the state database.

```powershell
winget-source-builder list-builds `
  --state-dir <dir> `
  [--limit <n>] `
  [--status <running|staged|published|failed>] `
  [--json]
```

| Option | Description |
|--------|-------------|
| `--limit` | Maximum number of builds to show (default: 20) |
| `--status` | Filter by build status |

**Examples:**

```powershell
# Show last 10 builds
winget-source-builder list-builds --state-dir ./state --limit 10

# Show only published builds
winget-source-builder list-builds `
  --state-dir ./state `
  --status published
```

#### `show`

**Purpose:** Inspect builds, packages, versions, or installer hashes from state.

```powershell
# Show build details
winget-source-builder show build --state-dir <dir> <build-id> [--json]

# Show package details
winget-source-builder show package --state-dir <dir> <package-id> [--json]

# Show version details
winget-source-builder show version `
  --state-dir <dir> `
  (--version-dir <dir> | --package-id <id> --version <ver>) `
  [--json]

# Show installer details
winget-source-builder show installer --state-dir <dir> <installer-hash> [--json]
```

**Examples:**

```powershell
# Show package information
winget-source-builder show package --state-dir ./state Vendor.App

# Show version as JSON
winget-source-builder show version `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.2.3 `
  --json

# Show build details
winget-source-builder show build --state-dir ./state 42
```

#### `verify`

**Purpose:** Check staged or published output against tracked state.

**When to use it:** To ensure output integrity before or after deployment.

```powershell
# Verify staged build
winget-source-builder verify staged `
  --state-dir <dir> `
  [--build-id <id>] `
  [--json]

# Verify published output
winget-source-builder verify published `
  --state-dir <dir> `
  --out-dir <dir> `
  [--json]
```

**Examples:**

```powershell
# Verify the staged build
winget-source-builder verify staged --state-dir ./state

# Verify specific published output
winget-source-builder verify published `
  --state-dir ./state `
  --out-dir ./publish
```

#### `hash`

**Purpose:** Print content and per-installer hashes for a repository target.

**When to use it:** Debugging hash mismatches or verifying manifest content.

```powershell
winget-source-builder hash `
  --repo-dir <dir> `
  (--version-dir <dir> | --package-id <id> --version <ver>) `
  [--json]
```

**Examples:**

```powershell
# Show hashes for a version
winget-source-builder hash `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3

# Output as JSON
winget-source-builder hash `
  --repo-dir ./manifests `
  --version-dir ./manifests/v/Vendor/App/1.2.3 `
  --json
```

#### `merge`

**Purpose:** Print the merged manifest for a repository target in canonical form.

**When to use it:** Debugging multi-file manifest merges or seeing the final merged output.

```powershell
winget-source-builder merge `
  --repo-dir <dir> `
  (--version-dir <dir> | --package-id <id> --version <ver>) `
  [--output-file <file>] `
  [--json]
```

**Examples:**

```powershell
# Print merged manifest to stdout
winget-source-builder merge `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3

# Save to file
winget-source-builder merge `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3 `
  --output-file ./merged.yaml
```

---

### Maintenance

Commands for cleanup and diagnostics.

#### `clean`

**Purpose:** Remove derived data to free space or reset state.

**When to use it:** Periodically to reclaim disk space, or when troubleshooting state issues.

```powershell
winget-source-builder clean `
  --state-dir <dir> `
  [--staging] `
  [--builds] `
  [--validation-queue] `
  [--published-tracking] `
  [--backend-cache] `
  [--all] `
  [--keep-last <n>] `
  [--older-than <duration>] `
  [--dry-run] `
  [--force]
```

| Option | Description |
|--------|-------------|
| `--staging` | Clean staged build directories |
| `--builds` | Clean build history |
| `--validation-queue` | Clean validation queue files |
| `--published-tracking` | Clean published build tracking |
| `--backend-cache` | Clean backend-specific caches |
| `--all` | Select all cleanable data (except working state) |
| `--keep-last` | Keep N most recent items when cleaning builds |
| `--older-than` | Only remove items older than duration (e.g., `7d`, `24h`) |

**Examples:**

```powershell
# Clean old staging directories
winget-source-builder clean --state-dir ./state --staging

# Keep only last 5 builds
winget-source-builder clean `
  --state-dir ./state `
  --builds `
  --keep-last 5

# Clean everything except working state
winget-source-builder clean --state-dir ./state --all

# Preview what would be cleaned
winget-source-builder clean `
  --state-dir ./state `
  --all `
  --dry-run
```

#### `doctor`

**Purpose:** Check environment, packaging assets, backend/index compatibility, and state health.

**When to use it:** First step when troubleshooting issues, or as a pre-flight check in CI.

```powershell
winget-source-builder doctor `
  [--repo-dir <dir>] `
  [--state-dir <dir>] `
  [--packaging-assets-dir <dir>] `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--json]
```

**Examples:**

```powershell
# Basic health check
winget-source-builder doctor

# Full check with paths
winget-source-builder doctor `
  --repo-dir ./manifests `
  --state-dir ./state `
  --packaging-assets-dir ./packaging

# JSON output for CI
winget-source-builder doctor --json > health-check.json
```

---

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | General error |
| `2` | Invalid arguments or usage |
| `3` | Repository or state not found |
| `4` | Backend unavailable |
| `5` | Validation failed |
| `6` | Output directory drift detected (publish) |
| `7` | Signing failed |
| `8` | State corruption detected |
