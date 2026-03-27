mod adapter;
mod backend;
mod builder;
mod i18n;
mod manifest;
#[cfg(any(not(windows), test))]
mod mszip;
mod progress;
mod state;
mod version;

rust_i18n::i18n!("locales", fallback = "en");

use std::path::PathBuf;
use std::process::ExitCode;
use std::{env, io::Write};

use clap::{Parser, Subcommand, ValueEnum};
use env_logger::{Builder, Target};
use i18n::Messages;
use log::{LevelFilter, error};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Static WinGet source builder for third-party repositories"
)]
struct Cli {
    #[arg(
        long,
        global = true,
        env = "WINGET_SOURCE_BUILDER_LANG",
        default_value = "en",
        value_name = "locale",
        help = "Locale tag for user-facing messages. Add more locales by adding files under locales/."
    )]
    lang: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Scan a manifest repo, update working state, and stage a publishable build")]
    Build(BuildArgs),
    #[command(about = "Package a staged build and write the final publish tree")]
    Publish(PublishArgs),
    #[command(about = "Compare the current repo contents against working state")]
    Diff(DiffArgs),
    #[command(about = "Show a summary of working state, staged builds, and published state")]
    Status(StatusArgs),
    #[command(about = "List recorded build executions from the state database")]
    ListBuilds(ListBuildsArgs),
    #[command(about = "Incrementally add selected manifests or versions into working state")]
    Add(TargetMutationArgs),
    #[command(
        alias = "delete",
        about = "Incrementally remove selected manifests or versions from working state"
    )]
    Remove(TargetMutationArgs),
    #[command(about = "Inspect builds, packages, versions, or installer hashes from state")]
    Show(ShowArgs),
    #[command(about = "Verify staged or published output against tracked state")]
    Verify(VerifyArgs),
    #[command(about = "Prune staged builds, history, queues, or cached backend artifacts")]
    Clean(CleanArgs),
    #[command(about = "Check environment, packaging assets, and backend/index compatibility")]
    Doctor(DoctorArgs),
    #[command(about = "Print the merged manifest for a selected repo target")]
    Merge(RepoTargetArgs),
    #[command(about = "Print content and installer hashes for a selected repo target")]
    Hash(HashArgs),
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct BuildArgs {
    /// Manifest repository root or manifests directory to scan.
    #[arg(long = "repo-dir")]
    pub(crate) repo_dir: PathBuf,

    /// State directory containing state.sqlite, staging, and validation queue data.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Restrict the build to one or more package identifiers.
    #[arg(long = "package-id")]
    pub(crate) package_ids: Vec<String>,

    /// Restrict the build to one or more version directories.
    #[arg(long = "version-dir")]
    pub(crate) version_dirs: Vec<PathBuf>,

    /// Source index version to stage.
    #[arg(long = "index-version", default_value = "v2")]
    pub(crate) index_version: CatalogFormat,

    /// Backend used to update the index database.
    #[arg(long, default_value = "wingetutil")]
    pub(crate) backend: BackendKind,

    /// Recompute all versions and restage the build even if no semantic changes are detected.
    #[arg(long)]
    pub(crate) force: bool,

    /// Report changes without mutating working state or staging output.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Skip writing validation-queue.json for this build.
    #[arg(long = "no-validation-queue")]
    pub(crate) no_validation_queue: bool,

    /// Strategy for resolving conflicting ARP DisplayVersion values within a package.
    #[arg(long = "display-version-conflict-strategy", default_value = "latest")]
    pub(crate) display_version_conflict_strategy: DisplayVersionConflictStrategy,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct PublishArgs {
    /// State directory containing staged builds.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Final publish output directory.
    #[arg(long = "out-dir")]
    pub(crate) out_dir: PathBuf,

    /// Directory containing AppxManifest.xml and Assets/.
    #[arg(long = "packaging-assets-dir")]
    pub(crate) packaging_assets_dir: PathBuf,

    /// Specific staged build id to publish. Defaults to the latest staged build.
    #[arg(long = "build-id")]
    pub(crate) build_id: Option<i64>,

    /// Overwrite an output tree even if it drifts from tracked published state.
    #[arg(long)]
    pub(crate) force: bool,

    /// Validate the staged build and output tree without packaging or copying files.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Optional PFX file used to sign the packaged MSIX.
    /// Windows uses signtool; Linux and macOS use makemsix sign when supported.
    #[arg(long = "sign-pfx-file")]
    pub(crate) sign_pfx_file: Option<PathBuf>,

    /// Optional password for the signing PFX file.
    #[arg(long = "sign-password")]
    pub(crate) sign_password: Option<String>,

    /// Environment variable name that contains the signing PFX password.
    #[arg(long = "sign-password-env")]
    pub(crate) sign_password_env: Option<String>,

    /// Optional RFC 3161 timestamp server URL for signing.
    /// Currently supported only by the Windows signtool path.
    #[arg(long = "timestamp-url")]
    pub(crate) timestamp_url: Option<String>,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct DiffArgs {
    /// Manifest repository root or manifests directory to scan.
    #[arg(long = "repo-dir")]
    pub(crate) repo_dir: PathBuf,

    /// State directory to compare against.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Restrict the diff to one or more package identifiers.
    #[arg(long = "package-id")]
    pub(crate) package_ids: Vec<String>,

    /// Restrict the diff to one or more version directories.
    #[arg(long = "version-dir")]
    pub(crate) version_dirs: Vec<PathBuf>,

    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct TargetMutationArgs {
    /// Manifest repository root or manifests directory to scan.
    #[arg(long = "repo-dir")]
    pub(crate) repo_dir: PathBuf,

    /// State directory to mutate.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// One or more version directories to add or remove.
    #[arg(long = "version-dir")]
    pub(crate) version_dirs: Vec<PathBuf>,

    /// One or more manifest files whose containing version directories should be targeted.
    #[arg(long = "manifest-file")]
    pub(crate) manifest_files: Vec<PathBuf>,

    /// Package identifier to target when selecting by package and version.
    #[arg(long = "package-id")]
    pub(crate) package_id: Option<String>,

    /// Package version to target when selecting by package and version.
    #[arg(long = "version")]
    pub(crate) version: Option<String>,

    /// Source index version to use. Defaults to the staged state value when available.
    #[arg(long = "index-version")]
    pub(crate) index_version: Option<CatalogFormat>,

    /// Backend to use. Defaults to the staged state value when available.
    #[arg(long)]
    pub(crate) backend: Option<BackendKind>,

    /// Recompute targeted versions even if hashes already match.
    #[arg(long)]
    pub(crate) force: bool,

    /// Show targeted changes without mutating working state.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Skip writing validation-queue.json for this mutation.
    #[arg(long = "no-validation-queue")]
    pub(crate) no_validation_queue: bool,

    /// Strategy for resolving conflicting ARP DisplayVersion values within a package.
    #[arg(long = "display-version-conflict-strategy", default_value = "latest")]
    pub(crate) display_version_conflict_strategy: DisplayVersionConflictStrategy,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct StatusArgs {
    /// State directory to inspect.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Optional repo directory to scan for a pending diff summary.
    #[arg(long = "repo-dir")]
    pub(crate) repo_dir: Option<PathBuf>,

    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct ListBuildsArgs {
    /// State directory to inspect.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Maximum number of build records to return.
    #[arg(long, default_value_t = 20)]
    pub(crate) limit: usize,

    /// Restrict output to one or more build statuses.
    #[arg(long = "status")]
    pub(crate) statuses: Vec<BuildRecordStatusFilter>,

    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct ShowArgs {
    /// Specific state object to inspect.
    #[command(subcommand)]
    pub(crate) command: ShowCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub(crate) enum ShowCommand {
    #[command(about = "Show one recorded build")]
    Build(ShowBuildArgs),
    #[command(about = "Show one package and its tracked versions")]
    Package(ShowPackageArgs),
    #[command(about = "Show one tracked version")]
    Version(ShowVersionArgs),
    #[command(about = "Show tracked versions that contain a given installer hash")]
    Installer(ShowInstallerArgs),
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct ShowBuildArgs {
    /// State directory to inspect.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Build id to show.
    pub(crate) build_id: i64,

    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct ShowPackageArgs {
    /// State directory to inspect.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Package identifier to show.
    pub(crate) package_id: String,

    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct ShowVersionArgs {
    /// State directory to inspect.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Version directory key to show.
    #[arg(long = "version-dir")]
    pub(crate) version_dir: Option<PathBuf>,

    /// Package identifier to use when selecting by package and version.
    #[arg(long = "package-id")]
    pub(crate) package_id: Option<String>,

    /// Package version to use when selecting by package and version.
    #[arg(long = "version")]
    pub(crate) version: Option<String>,

    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct ShowInstallerArgs {
    /// State directory to inspect.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Installer SHA-256 identity to search for.
    pub(crate) installer_hash: String,

    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct VerifyArgs {
    /// Verify a staged build or a published output tree.
    #[command(subcommand)]
    pub(crate) command: VerifyCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub(crate) enum VerifyCommand {
    #[command(about = "Verify a staged build tree under state-dir/staging")]
    Staged(VerifyStagedArgs),
    #[command(about = "Verify a published output directory against tracked published state")]
    Published(VerifyPublishedArgs),
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct VerifyStagedArgs {
    /// State directory to inspect.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Specific staged build id to verify. Defaults to the latest staged build.
    #[arg(long = "build-id")]
    pub(crate) build_id: Option<i64>,

    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct VerifyPublishedArgs {
    /// State directory to inspect.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Published output directory to verify.
    #[arg(long = "out-dir")]
    pub(crate) out_dir: PathBuf,

    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct CleanArgs {
    /// State directory to clean.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: PathBuf,

    /// Remove staged build directories.
    #[arg(long)]
    pub(crate) staging: bool,

    /// Prune build history records.
    #[arg(long)]
    pub(crate) builds: bool,

    /// Delete validation-queue.json.
    #[arg(long = "validation-queue")]
    pub(crate) validation_queue: bool,

    /// Clear published tracking metadata from state.sqlite.
    #[arg(long = "published-tracking")]
    pub(crate) published_tracking: bool,

    /// Remove backend cache artifacts such as mutable WinGetUtil databases.
    #[arg(long = "backend-cache")]
    pub(crate) backend_cache: bool,

    /// Select all supported clean targets.
    #[arg(long)]
    pub(crate) all: bool,

    /// Number of most recent items to keep when pruning staging or build records.
    #[arg(long = "keep-last", default_value_t = 0)]
    pub(crate) keep_last: usize,

    /// Delete only items older than this duration, for example 7d, 12h, or 30m.
    #[arg(long = "older-than")]
    pub(crate) older_than: Option<String>,

    /// Report cleanup actions without deleting anything.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Required for destructive cleanup of published tracking.
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct DoctorArgs {
    /// Optional manifest repository root to inspect.
    #[arg(long = "repo-dir")]
    pub(crate) repo_dir: Option<PathBuf>,

    /// Optional state directory to inspect.
    #[arg(long = "state-dir")]
    pub(crate) state_dir: Option<PathBuf>,

    /// Optional packaging assets directory to validate.
    #[arg(long = "packaging-assets-dir")]
    pub(crate) packaging_assets_dir: Option<PathBuf>,

    /// Optional backend to evaluate for compatibility.
    #[arg(long)]
    pub(crate) backend: Option<BackendKind>,

    /// Optional index version to evaluate for compatibility.
    #[arg(long = "index-version")]
    pub(crate) index_version: Option<CatalogFormat>,

    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct RepoTargetArgs {
    /// Manifest repository root or manifests directory to scan.
    #[arg(long = "repo-dir")]
    pub(crate) repo_dir: PathBuf,

    /// Version directory to target.
    #[arg(long = "version-dir")]
    pub(crate) version_dir: Option<PathBuf>,

    /// Manifest file whose containing version directory should be targeted.
    #[arg(long = "manifest-file")]
    pub(crate) manifest_file: Option<PathBuf>,

    /// Package identifier to target when selecting by package and version.
    #[arg(long = "package-id")]
    pub(crate) package_id: Option<String>,

    /// Package version to target when selecting by package and version.
    #[arg(long = "version")]
    pub(crate) version: Option<String>,

    /// Optional file path to write the output instead of printing to stdout.
    #[arg(long = "output-file")]
    pub(crate) output_file: Option<PathBuf>,

    /// Emit machine-readable JSON instead of text or YAML.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct HashArgs {
    /// Repo target to hash.
    #[command(flatten)]
    pub(crate) target: RepoTargetArgs,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub(crate) enum CatalogFormat {
    /// Build legacy source.msix output.
    V1,
    /// Build modern source2.msix output.
    V2,
}

impl CatalogFormat {
    pub(crate) const fn package_file_name(self) -> &'static str {
        match self {
            Self::V1 => "source.msix",
            Self::V2 => "source2.msix",
        }
    }

    pub(crate) const fn wingetutil_schema_version(self) -> (u32, u32) {
        match self {
            Self::V1 => (1, u32::MAX),
            Self::V2 => (2, 0),
        }
    }

    pub(crate) const fn uses_package_sidecars(self) -> bool {
        matches!(self, Self::V2)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub(crate) enum BackendKind {
    /// Use WinGetUtil.dll to mutate and package the index database.
    Wingetutil,
    /// Use the custom Rust backend to build the published index.
    Rust,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub(crate) enum DisplayVersionConflictStrategy {
    /// Keep the conflicting DisplayVersion on the highest PackageVersion.
    Latest,
    /// Keep the conflicting DisplayVersion on the lowest PackageVersion.
    Oldest,
    /// Remove the conflicting DisplayVersion from every contender.
    StripAll,
    /// Fail the command instead of rewriting the manifests.
    Error,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub(crate) enum BuildRecordStatusFilter {
    /// Build record created but not yet completed.
    Running,
    /// Build successfully staged but not yet published.
    Staged,
    /// Build successfully published.
    Published,
    /// Build failed before staging or publishing completed.
    Failed,
}

fn main() -> ExitCode {
    init_logging();
    let cli = Cli::parse();
    let messages = Messages::new(cli.lang);

    let result = match cli.command {
        Command::Build(args) => builder::run_build(args, messages.clone()),
        Command::Publish(args) => builder::run_publish(args, messages.clone()),
        Command::Diff(args) => builder::run_diff(args, messages.clone()),
        Command::Status(args) => builder::run_status(args, messages.clone()),
        Command::ListBuilds(args) => builder::run_list_builds(args, messages.clone()),
        Command::Add(args) => builder::run_add(args, messages.clone()),
        Command::Remove(args) => builder::run_remove(args, messages.clone()),
        Command::Show(args) => builder::run_show(args, messages.clone()),
        Command::Verify(args) => builder::run_verify(args, messages.clone()),
        Command::Clean(args) => builder::run_clean(args, messages.clone()),
        Command::Doctor(args) => builder::run_doctor(args, messages.clone()),
        Command::Merge(args) => builder::run_merge(args, messages.clone()),
        Command::Hash(args) => builder::run_hash(args, messages.clone()),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!("{}", messages.build_failed(&error));
            ExitCode::FAILURE
        }
    }
}

fn init_logging() {
    let mut builder = Builder::new();
    if let Ok(filters) = env::var("WINGET_SOURCE_BUILDER_LOG").or_else(|_| env::var("RUST_LOG")) {
        builder.parse_filters(&filters);
    } else {
        builder.filter_level(LevelFilter::Info);
    }

    builder
        .target(Target::Stdout)
        .format(|buf, record| writeln!(buf, "[{}] {}", record.level(), record.args()))
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_build_with_index_version_and_conflict_strategy() {
        let cli = Cli::try_parse_from([
            "winget-source-builder",
            "build",
            "--repo-dir",
            "repo",
            "--state-dir",
            "state",
            "--index-version",
            "v2",
            "--display-version-conflict-strategy",
            "strip-all",
            "--package-id",
            "Example.App",
            "--version-dir",
            "repo/manifests/e/Example/App/1.0.0",
        ])
        .expect("new CLI flags should parse");

        match cli.command {
            Command::Build(args) => {
                assert_eq!(args.index_version, CatalogFormat::V2);
                assert_eq!(
                    args.display_version_conflict_strategy,
                    DisplayVersionConflictStrategy::StripAll
                );
                assert_eq!(args.package_ids, vec!["Example.App"]);
                assert_eq!(
                    args.version_dirs,
                    vec![PathBuf::from("repo/manifests/e/Example/App/1.0.0")]
                );
            }
            _ => panic!("expected build command"),
        }
    }

    #[test]
    fn parses_remove_with_new_conflict_flag() {
        let cli = Cli::try_parse_from([
            "winget-source-builder",
            "remove",
            "--repo-dir",
            "repo",
            "--state-dir",
            "state",
            "--package-id",
            "Example.App",
            "--version",
            "1.0.0",
            "--display-version-conflict-strategy",
            "error",
        ])
        .expect("remove should accept the new conflict strategy flag");

        match cli.command {
            Command::Remove(args) => {
                assert_eq!(args.package_id.as_deref(), Some("Example.App"));
                assert_eq!(args.version.as_deref(), Some("1.0.0"));
                assert_eq!(
                    args.display_version_conflict_strategy,
                    DisplayVersionConflictStrategy::Error
                );
            }
            _ => panic!("expected remove command"),
        }
    }

    #[test]
    fn parses_diff_with_target_filters() {
        let cli = Cli::try_parse_from([
            "winget-source-builder",
            "diff",
            "--repo-dir",
            "repo",
            "--state-dir",
            "state",
            "--package-id",
            "Example.App",
            "--version-dir",
            "repo/manifests/e/Example/App/1.0.0",
            "--json",
        ])
        .expect("diff should accept the planned filters");

        match cli.command {
            Command::Diff(args) => {
                assert_eq!(args.package_ids, vec!["Example.App"]);
                assert_eq!(
                    args.version_dirs,
                    vec![PathBuf::from("repo/manifests/e/Example/App/1.0.0")]
                );
                assert!(args.json);
            }
            _ => panic!("expected diff command"),
        }
    }

    #[test]
    fn rejects_legacy_format_flag() {
        let error = Cli::try_parse_from([
            "winget-source-builder",
            "build",
            "--repo-dir",
            "repo",
            "--state-dir",
            "state",
            "--format",
            "v2",
        ])
        .expect_err("legacy --format should be rejected");
        let rendered = error.to_string();
        assert!(rendered.contains("--format"));
    }

    #[test]
    fn rejects_legacy_display_version_conflict_flag() {
        let error = Cli::try_parse_from([
            "winget-source-builder",
            "build",
            "--repo-dir",
            "repo",
            "--state-dir",
            "state",
            "--display-version-conflict",
            "latest",
        ])
        .expect_err("legacy --display-version-conflict should be rejected");
        let rendered = error.to_string();
        assert!(rendered.contains("--display-version-conflict"));
    }

    #[test]
    fn parses_clean_with_older_than() {
        let cli = Cli::try_parse_from([
            "winget-source-builder",
            "clean",
            "--state-dir",
            "state",
            "--staging",
            "--older-than",
            "7d",
        ])
        .expect("clean should accept --older-than");

        match cli.command {
            Command::Clean(args) => {
                assert_eq!(args.older_than.as_deref(), Some("7d"));
            }
            _ => panic!("expected clean command"),
        }
    }
}
