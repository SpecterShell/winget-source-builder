use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail, ensure};
use log::info;
use rayon::prelude::*;
use serde_json::json;
use walkdir::WalkDir;

use crate::adapter::{
    AdapterOperation, AdapterRequest, absolute_string, package_published_index, run_adapter,
    sign_published_index,
};
use crate::backend::run_rust_backend;
use crate::i18n::Messages;
use crate::manifest::{
    ComputedVersionSnapshot, ManifestWarning, ValidationRequirement, added_installers,
    compute_version_snapshot_with_warnings, extract_display_versions_from_manifest_bytes,
    installer_records_to_json, normalize_rel, parse_installer_records_json,
    retain_display_versions_in_snapshot, scan_root, sha256_bytes,
};
use crate::progress::ProgressReporter;
use crate::state::{
    BuildPackageChange, BuildRecord, BuildVersionChange, PublishedFile, StateStore, StoredFile,
    StoredPackage, StoredVersion, WorkingStateUpdate,
};
use crate::version::compare_versions;
use crate::{
    BackendKind, BuildArgs, BuildRecordStatusFilter, CatalogFormat, CleanArgs, DiffArgs,
    DisplayVersionConflictStrategy, DoctorArgs, HashArgs, ListBuildsArgs, PublishArgs,
    RepoTargetArgs, ShowArgs, ShowBuildArgs, ShowCommand, ShowInstallerArgs, ShowPackageArgs,
    ShowVersionArgs, StatusArgs, TargetMutationArgs, VerifyArgs, VerifyCommand,
    VerifyPublishedArgs, VerifyStagedArgs,
};

#[derive(Debug, Clone)]
struct CurrentFileScan {
    path: String,
    version_dir: String,
    abs_path: PathBuf,
    version_dir_abs: PathBuf,
    size: u64,
    mtime_ns: i64,
    raw_sha256: Vec<u8>,
}

#[derive(Debug, Clone)]
struct StagedPackageFile {
    relpath: String,
    sha256: Vec<u8>,
}

#[derive(Debug, Clone)]
enum VersionSemanticChange {
    Add(Box<ComputedVersionSnapshot>),
    Update {
        old: Box<StoredVersion>,
        new: Box<ComputedVersionSnapshot>,
    },
    Remove(Box<StoredVersion>),
    Noop,
}

#[derive(Debug, Clone, Copy)]
struct DiffSummary {
    added_versions: usize,
    updated_versions: usize,
    removed_versions: usize,
    installer_revalidations: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
struct PublishTreeDrift {
    tracked_files: usize,
    missing_files: Vec<String>,
    mismatched_hash_files: Vec<String>,
    extra_files: Vec<String>,
}

impl PublishTreeDrift {
    fn has_drift(&self) -> bool {
        !self.missing_files.is_empty()
            || !self.mismatched_hash_files.is_empty()
            || !self.extra_files.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
struct TargetSelectionSpec {
    version_dirs: Vec<PathBuf>,
    manifest_files: Vec<PathBuf>,
    package_ids: Vec<String>,
    package_id: Option<String>,
    version: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
struct BuildExecutionOptions<'a> {
    target_spec: Option<&'a TargetSelectionSpec>,
    remove_selected: bool,
    write_validation_queue_file: bool,
    dry_run: bool,
}

/// Scans the repo, updates working state, and stages the next publishable build.
///
/// # Arguments
///
/// * `args` - Build-time repo, state, backend, filtering, and conflict-strategy options.
/// * `messages` - Localized status and progress strings for user-facing output.
pub fn run_build(args: BuildArgs, messages: Messages) -> Result<()> {
    info!(
        "{}",
        messages.build_started(&args.repo_dir, &args.state_dir)
    );
    let mut state = StateStore::open(&args.state_dir)?;
    let started_unix = unix_now()?;
    let build_id = state.begin_build(started_unix)?;
    let target_spec = if args.package_ids.is_empty() && args.version_dirs.is_empty() {
        None
    } else {
        Some(TargetSelectionSpec {
            version_dirs: args.version_dirs.clone(),
            manifest_files: Vec::new(),
            package_ids: args.package_ids.clone(),
            package_id: None,
            version: None,
        })
    };

    let result = run_build_inner(
        &args,
        &mut state,
        build_id,
        messages,
        BuildExecutionOptions {
            target_spec: target_spec.as_ref(),
            remove_selected: false,
            write_validation_queue_file: !args.no_validation_queue,
            dry_run: args.dry_run,
        },
    );
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            state.mark_build_failed(
                build_id,
                unix_now().unwrap_or(started_unix),
                &format!("{error:#}"),
            )?;
            Err(error)
        }
    }
}

/// Applies a targeted add mutation and restages the resulting build.
///
/// # Arguments
///
/// * `args` - Target selectors and mutation settings for the addressed manifests.
/// * `messages` - Localized status and progress strings for user-facing output.
pub fn run_add(args: TargetMutationArgs, messages: Messages) -> Result<()> {
    run_targeted_mutation(args, messages, false)
}

/// Applies a targeted remove mutation and restages the resulting build.
///
/// # Arguments
///
/// * `args` - Target selectors and mutation settings for the addressed manifests.
/// * `messages` - Localized status and progress strings for user-facing output.
pub fn run_remove(args: TargetMutationArgs, messages: Messages) -> Result<()> {
    run_targeted_mutation(args, messages, true)
}

fn run_targeted_mutation(
    args: TargetMutationArgs,
    messages: Messages,
    remove_selected: bool,
) -> Result<()> {
    let mut state = StateStore::open(&args.state_dir)?;
    let backend = args
        .backend
        .or(state.last_staged_backend()?)
        .unwrap_or(BackendKind::Wingetutil);
    let index_version = args
        .index_version
        .or(state.last_staged_index_version()?)
        .unwrap_or(CatalogFormat::V2);
    let build_args = BuildArgs {
        repo_dir: args.repo_dir,
        state_dir: args.state_dir,
        package_ids: Vec::new(),
        version_dirs: Vec::new(),
        index_version,
        backend,
        force: args.force,
        dry_run: args.dry_run,
        no_validation_queue: args.no_validation_queue,
        display_version_conflict_strategy: args.display_version_conflict_strategy,
    };
    let target_spec = TargetSelectionSpec {
        version_dirs: args.version_dirs,
        manifest_files: args.manifest_files,
        package_ids: Vec::new(),
        package_id: args.package_id,
        version: args.version,
    };
    validate_target_selection_spec(&target_spec)?;

    let started_unix = unix_now()?;
    let build_id = state.begin_build(started_unix)?;
    let result = run_build_inner(
        &build_args,
        &mut state,
        build_id,
        messages,
        BuildExecutionOptions {
            target_spec: Some(&target_spec),
            remove_selected,
            write_validation_queue_file: !build_args.no_validation_queue,
            dry_run: build_args.dry_run,
        },
    );
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            state.mark_build_failed(
                build_id,
                unix_now().unwrap_or(started_unix),
                &format!("{error:#}"),
            )?;
            Err(error)
        }
    }
}

/// Packages a staged build into the final publish tree and optionally signs the catalog MSIX.
///
/// # Arguments
///
/// * `args` - Staged-build selection, output, packaging-assets, and signing options.
/// * `messages` - Localized status and progress strings for user-facing output.
pub fn run_publish(args: PublishArgs, messages: Messages) -> Result<()> {
    info!(
        "{}",
        messages.publish_started(&args.state_dir, &args.out_dir)
    );
    let mut state = StateStore::open(&args.state_dir)?;
    let latest_staged_build_id = state.last_staged_build_id()?.ok_or_else(|| {
        anyhow!(
            "no staged build is available in {}",
            args.state_dir.display()
        )
    })?;
    let build_id = args.build_id.unwrap_or(latest_staged_build_id);
    let stage_root = state.stage_root_for_build(build_id);
    ensure!(
        stage_root.is_dir(),
        "staged build directory is missing: {}",
        stage_root.display()
    );
    let index_version = if build_id == latest_staged_build_id {
        state.last_staged_index_version()?.ok_or_else(|| {
            anyhow!(
                "latest staged build metadata is missing index version in {}",
                args.state_dir.display()
            )
        })?
    } else {
        infer_staged_index_version(&stage_root)?
    };

    let packaging_assets_root =
        crate::adapter::resolve_packaging_assets_root(Some(&args.packaging_assets_dir), None)?;
    let publish_db_path = stage_root.join("index-publish.db");
    ensure!(
        publish_db_path.is_file(),
        "staged published index DB is missing: {}",
        publish_db_path.display()
    );

    let previous_published_files = state.load_published_files_current()?;
    let output_drift = collect_publish_tree_drift(&args.out_dir, &previous_published_files)?;
    if !args.force && output_drift.has_drift() {
        bail!(
            "existing output directory {} does not match tracked published state; rerun publish with --force to overwrite it",
            args.out_dir.display()
        );
    }
    if args.dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "build_id": build_id,
                "index_version": index_version.as_str(),
                "stage_root": stage_root,
                "publish_db_path": publish_db_path,
                "output_dir": args.out_dir,
                "output_drift": output_drift,
            }))?
        );
        return Ok(());
    }

    let progress = ProgressReporter::new();
    let package_progress = progress.spinner(messages.progress_packaging_publish());
    package_published_index(
        &packaging_assets_root,
        &stage_root,
        &publish_db_path,
        index_version,
    )?;
    if let Some(sign_pfx_file) = args.sign_pfx_file.as_deref() {
        let password = resolve_sign_password(
            args.sign_password.as_deref(),
            args.sign_password_env.as_deref(),
        )?;
        let staged_catalog = stage_root.join(index_version.package_file_name());
        sign_published_index(
            &staged_catalog,
            sign_pfx_file,
            password.as_deref(),
            args.timestamp_url.as_deref(),
        )?;
    } else if args.sign_password.is_some()
        || args.sign_password_env.is_some()
        || args.timestamp_url.is_some()
    {
        bail!("--sign-pfx-file is required when signing-related publish options are used");
    }
    ProgressReporter::finish(package_progress);

    let versions = state.load_versions_current()?;
    let packages = state.load_packages_current()?;
    let catalog_package_name = index_version.package_file_name();

    let commit_progress = progress.spinner(messages.progress_committing_output());
    commit_staged_output_tree(
        &stage_root,
        &args.out_dir,
        &previous_published_files,
        &versions,
        &packages,
        catalog_package_name,
    )?;
    ProgressReporter::finish(commit_progress);

    let staged_catalog = stage_root.join(catalog_package_name);
    let catalog_hash = sha256_bytes(
        &fs::read(&staged_catalog)
            .with_context(|| format!("failed to read {}", staged_catalog.display()))?,
    );
    let final_published_files =
        build_published_files(&versions, &packages, catalog_package_name, catalog_hash);
    state.replace_published_state(build_id, unix_now()?, &final_published_files)?;
    info!(
        "{}",
        messages.publish_completed(&args.out_dir, &args.state_dir)
    );
    Ok(())
}

/// Compares the repo against working state and reports semantic version-level changes.
///
/// # Arguments
///
/// * `args` - Repo, state, filtering, and output-format options for the comparison.
/// * `messages` - Localized strings used while collecting diff data.
pub fn run_diff(args: DiffArgs, messages: Messages) -> Result<()> {
    let state = StateStore::open(&args.state_dir)?;
    let target_spec = if args.package_ids.is_empty() && args.version_dirs.is_empty() {
        None
    } else {
        Some(TargetSelectionSpec {
            version_dirs: args.version_dirs.clone(),
            manifest_files: Vec::new(),
            package_ids: args.package_ids.clone(),
            package_id: None,
            version: None,
        })
    };
    let summary = collect_diff_summary(&args.repo_dir, &state, &messages, target_spec.as_ref())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "added_versions": summary.added_versions,
                "updated_versions": summary.updated_versions,
                "removed_versions": summary.removed_versions,
                "installer_revalidations": summary.installer_revalidations,
            }))?
        );
    } else {
        println!("Added versions: {}", summary.added_versions);
        println!("Updated versions: {}", summary.updated_versions);
        println!("Removed versions: {}", summary.removed_versions);
        println!(
            "Installer revalidations: {}",
            summary.installer_revalidations
        );
    }

    Ok(())
}

/// Summarizes working state, staged and published build pointers, and optional pending diff data.
///
/// # Arguments
///
/// * `args` - State location plus optional repo and JSON-output settings.
/// * `messages` - Localized strings used if a pending diff must be computed.
pub fn run_status(args: StatusArgs, messages: Messages) -> Result<()> {
    let state = StateStore::open(&args.state_dir)?;
    let (file_count, version_count, package_count, published_file_count) =
        state.current_counts()?;
    let validation_queue_count = count_validation_queue_items(&state.validation_queue_path())?;
    let diff_summary = if let Some(repo_dir) = args.repo_dir.as_ref() {
        Some(collect_diff_summary(repo_dir, &state, &messages, None)?)
    } else {
        None
    };

    let summary = json!({
        "state_dir": args.state_dir,
        "last_staged_build_id": state.last_staged_build_id()?,
        "last_staged_index_version": state.last_staged_index_version()?.map(CatalogFormat::as_str),
        "last_staged_backend": state.last_staged_backend()?.map(BackendKind::as_str),
        "last_published_build_id": state.last_published_build_id()?,
        "last_successful_unix_epoch": state.last_successful_build_epoch()?,
        "current_counts": {
            "files": file_count,
            "versions": version_count,
            "packages": package_count,
            "published_files": published_file_count,
        },
        "validation_queue_items": validation_queue_count,
        "pending_diff": diff_summary.map(|summary| json!({
            "added_versions": summary.added_versions,
            "updated_versions": summary.updated_versions,
            "removed_versions": summary.removed_versions,
            "installer_revalidations": summary.installer_revalidations,
        })),
    });

    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!("State directory: {}", args.state_dir.display());
        println!("Last staged build: {:?}", state.last_staged_build_id()?);
        println!(
            "Last published build: {:?}",
            state.last_published_build_id()?
        );
        println!(
            "Staged backend/index: {:?} / {:?}",
            state.last_staged_backend()?,
            state.last_staged_index_version()?
        );
        println!(
            "Working state: {} files, {} versions, {} packages",
            file_count, version_count, package_count
        );
        println!("Published files tracked: {}", published_file_count);
        println!("Validation queue items: {}", validation_queue_count);
        if let Some(summary) = diff_summary {
            println!(
                "Pending diff: +{} ~{} -{} | installer revalidations {}",
                summary.added_versions,
                summary.updated_versions,
                summary.removed_versions,
                summary.installer_revalidations
            );
        }
    }

    Ok(())
}

/// Lists recorded build attempts from the state store, newest first.
///
/// # Arguments
///
/// * `args` - State location and optional history filters such as status and limit.
/// * `_messages` - Unused localized message bundle kept for a consistent command signature.
pub fn run_list_builds(args: ListBuildsArgs, _messages: Messages) -> Result<()> {
    let state = StateStore::open(&args.state_dir)?;
    let builds = filter_build_records(state.load_builds(Some(args.limit))?, &args.statuses);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&builds)?);
    } else if builds.is_empty() {
        println!("No builds found.");
    } else {
        for build in builds {
            let finished = build
                .finished_at_unix
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "#{:>4}  {:<13} started={} finished={}{}",
                build.build_id,
                build.status,
                build.started_at_unix,
                finished,
                build
                    .error_text
                    .as_deref()
                    .map(|error| format!(" error={error}"))
                    .unwrap_or_default()
            );
        }
    }

    Ok(())
}

/// Dispatches `show` subcommands for build, package, version, and installer inspection.
///
/// # Arguments
///
/// * `args` - Selected `show` subcommand and its target arguments.
/// * `_messages` - Unused localized message bundle kept for a consistent command signature.
pub fn run_show(args: ShowArgs, _messages: Messages) -> Result<()> {
    match args.command {
        ShowCommand::Build(args) => run_show_build(args),
        ShowCommand::Package(args) => run_show_package(args),
        ShowCommand::Version(args) => run_show_version(args),
        ShowCommand::Installer(args) => run_show_installer(args),
    }
}

/// Dispatches verification of staged artifacts or the published output tree.
///
/// # Arguments
///
/// * `args` - Selected verification scope and its target arguments.
/// * `_messages` - Unused localized message bundle kept for a consistent command signature.
pub fn run_verify(args: VerifyArgs, _messages: Messages) -> Result<()> {
    match args.command {
        VerifyCommand::Staged(args) => run_verify_staged(args),
        VerifyCommand::Published(args) => run_verify_published(args),
    }
}

/// Removes derived state such as staging roots, old build records, and backend caches.
///
/// # Arguments
///
/// * `args` - Cleanup targets, retention rules, and safety flags.
/// * `_messages` - Unused localized message bundle kept for a consistent command signature.
pub fn run_clean(args: CleanArgs, _messages: Messages) -> Result<()> {
    let state = StateStore::open(&args.state_dir)?;
    let older_than = parse_older_than(args.older_than.as_deref())?;
    let older_than_description = args
        .older_than
        .as_deref()
        .map(|value| format!(", older than {value}"))
        .unwrap_or_default();
    let apply_all = args.all;
    let clean_staging = apply_all || args.staging;
    let clean_builds = apply_all || args.builds;
    let clean_validation_queue = apply_all || args.validation_queue;
    let clean_published_tracking = apply_all || args.published_tracking;
    let clean_backend_cache = apply_all || args.backend_cache;

    ensure!(
        clean_staging
            || clean_builds
            || clean_validation_queue
            || clean_published_tracking
            || clean_backend_cache,
        "no clean target selected"
    );

    if clean_published_tracking {
        ensure!(
            args.force,
            "--force is required to clear published tracking"
        );
    }

    let mut actions = Vec::<String>::new();
    if clean_staging {
        actions.push(format!(
            "prune staging, keep latest {}{}",
            args.keep_last, older_than_description
        ));
    }
    if clean_builds {
        actions.push(format!(
            "prune build records, keep latest {}{}",
            args.keep_last, older_than_description
        ));
    }
    if clean_validation_queue {
        actions.push("delete validation-queue.json".to_string());
    }
    if clean_published_tracking {
        actions.push("clear published tracking".to_string());
    }
    if clean_backend_cache {
        actions.push("delete backend cache under state/writer".to_string());
    }

    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&actions)?);
        return Ok(());
    }

    if clean_staging {
        prune_staging_dirs(state.staging_root(), args.keep_last, older_than)?;
    }
    if clean_builds {
        state.prune_build_records(args.keep_last, older_than.map(system_time_to_unix))?;
    }
    if clean_validation_queue {
        let queue = state.validation_queue_path();
        if queue.exists() {
            fs::remove_file(&queue)
                .with_context(|| format!("failed to remove {}", queue.display()))?;
        }
    }
    if clean_published_tracking {
        state.clear_published_tracking()?;
    }
    if clean_backend_cache {
        let writer_dir = args.state_dir.join("writer");
        if writer_dir.exists() {
            fs::remove_dir_all(&writer_dir)
                .with_context(|| format!("failed to remove {}", writer_dir.display()))?;
        }
    }

    Ok(())
}

/// Reports environment and configuration readiness for the requested backend and index version.
///
/// # Arguments
///
/// * `args` - Optional repo, state, packaging, backend, and index-version checks to run.
/// * `_messages` - Unused localized message bundle kept for a consistent command signature.
pub fn run_doctor(args: DoctorArgs, _messages: Messages) -> Result<()> {
    let state_ok = args
        .state_dir
        .as_ref()
        .map(|path| StateStore::open(path).is_ok())
        .unwrap_or(true);
    let repo_ok = args
        .repo_dir
        .as_ref()
        .map(|path| path.is_dir())
        .unwrap_or(true);
    let packaging_assets = args
        .packaging_assets_dir
        .as_ref()
        .map(|path| crate::adapter::resolve_packaging_assets_root(Some(path), None).is_ok())
        .unwrap_or(true);
    let backend = args.backend.unwrap_or(BackendKind::Wingetutil);
    let index_version = args.index_version.unwrap_or(CatalogFormat::V2);

    let report = json!({
        "repo_dir_ok": repo_ok,
        "state_dir_ok": state_ok,
        "packaging_assets_ok": packaging_assets,
        "runtime": {
            "wingetutil_available": crate::adapter::runtime_wingetutil_available(),
            "msix_packager": crate::adapter::runtime_msix_packager(),
            "msix_signer": crate::adapter::runtime_msix_signer(),
            "openssl": crate::adapter::runtime_openssl(),
        },
        "requested_backend": backend.as_str(),
        "requested_index_version": index_version.as_str(),
        "compatibility": {
            "backend_ok": match backend {
                BackendKind::Wingetutil => cfg!(windows),
                BackendKind::Rust => true,
            },
            "index_ok": match index_version {
                CatalogFormat::V1 => true,
                CatalogFormat::V2 => cfg!(windows) || backend == BackendKind::Rust,
            }
        }
    });

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Renders the merged manifest for a selected version directory in YAML or JSON form.
///
/// # Arguments
///
/// * `args` - Repo target selection plus optional output-file and JSON-format settings.
/// * `_messages` - Unused localized message bundle kept for a consistent command signature.
pub fn run_merge(args: RepoTargetArgs, _messages: Messages) -> Result<()> {
    let snapshot = compute_repo_target_snapshot(&args)?;
    if args.json {
        let yaml: serde_yaml::Value = serde_yaml::from_slice(&snapshot.published_manifest_bytes)
            .context("failed to parse merged manifest bytes")?;
        let output = serde_json::to_string_pretty(&yaml)?;
        if let Some(path) = &args.output_file {
            fs::write(path, output)
                .with_context(|| format!("failed to write {}", path.display()))?;
        } else {
            println!("{output}");
        }
    } else {
        let output = String::from_utf8(snapshot.published_manifest_bytes)
            .context("merged manifest is not valid UTF-8")?;
        if let Some(path) = &args.output_file {
            fs::write(path, output)
                .with_context(|| format!("failed to write {}", path.display()))?;
        } else {
            print!("{output}");
        }
    }
    Ok(())
}

/// Prints the hash identities derived from a selected merged manifest snapshot.
///
/// # Arguments
///
/// * `args` - Repo target selection and JSON-output settings for hash inspection.
/// * `_messages` - Unused localized message bundle kept for a consistent command signature.
pub fn run_hash(args: HashArgs, _messages: Messages) -> Result<()> {
    let snapshot = compute_repo_target_snapshot(&args.target)?;
    let report = json!({
        "version_dir": snapshot.version_dir,
        "package_id": snapshot.package_id,
        "package_version": snapshot.package_version,
        "channel": snapshot.channel,
        "version_content_sha256": hex::encode(&snapshot.version_content_sha256),
        "version_installer_sha256": hex::encode(&snapshot.version_installer_sha256),
        "published_manifest_sha256": hex::encode(&snapshot.published_manifest_sha256),
        "installers": snapshot.installers.iter().map(|installer| json!({
            "installer_sha256": installer.installer_sha256,
            "installer_url": installer.installer_url,
            "architecture": installer.architecture,
            "installer_type": installer.installer_type,
            "installer_locale": installer.installer_locale,
            "scope": installer.scope,
            "package_family_name": installer.package_family_name,
            "product_codes": installer.product_codes,
        })).collect::<Vec<_>>()
    });

    if args.target.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Version content hash: {}", report["version_content_sha256"]);
        println!(
            "Version installer hash: {}",
            report["version_installer_sha256"]
        );
        println!(
            "Published manifest hash: {}",
            report["published_manifest_sha256"]
        );
        println!(
            "Installer hashes: {}",
            report["installers"]
                .as_array()
                .unwrap_or(&Vec::new())
                .iter()
                .map(|installer| installer["installer_sha256"].as_str().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}

fn run_show_build(args: ShowBuildArgs) -> Result<()> {
    let state = StateStore::open(&args.state_dir)?;
    let build = state
        .load_build(args.build_id)?
        .ok_or_else(|| anyhow!("build {} not found", args.build_id))?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&build)?);
    } else {
        println!(
            "#{:>4}  {:<13} started={} finished={}{}",
            build.build_id,
            build.status,
            build.started_at_unix,
            build
                .finished_at_unix
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            build
                .error_text
                .as_deref()
                .map(|error| format!(" error={error}"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

fn run_show_package(args: ShowPackageArgs) -> Result<()> {
    let state = StateStore::open(&args.state_dir)?;
    let packages = state.load_packages_current()?;
    let package = packages
        .get(args.package_id.as_str())
        .ok_or_else(|| anyhow!("package {} not found", args.package_id))?;
    let versions = state
        .load_versions_current()?
        .into_values()
        .filter(|version| version.package_id == args.package_id)
        .collect::<Vec<_>>();
    let report = json!({
        "package": package,
        "versions": versions,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn run_show_version(args: ShowVersionArgs) -> Result<()> {
    let state = StateStore::open(&args.state_dir)?;
    let versions = state.load_versions_current()?;
    let version = resolve_state_version(
        &versions,
        args.version_dir.as_deref(),
        args.package_id.as_deref(),
        args.version.as_deref(),
    )?;
    println!("{}", serde_json::to_string_pretty(&version)?);
    Ok(())
}

fn run_show_installer(args: ShowInstallerArgs) -> Result<()> {
    let state = StateStore::open(&args.state_dir)?;
    let mut matches = Vec::new();
    for version in state.load_versions_current()?.into_values() {
        let Some(json) = version.installers_json.as_deref() else {
            continue;
        };
        let installers = parse_installer_records_json(Some(json))?;
        for installer in installers {
            if installer
                .installer_sha256
                .eq_ignore_ascii_case(&args.installer_hash)
            {
                matches.push(json!({
                    "version_dir": version.version_dir,
                    "package_id": version.package_id,
                    "package_version": version.package_version,
                    "channel": version.channel,
                    "installer": installer,
                }));
            }
        }
    }
    println!("{}", serde_json::to_string_pretty(&matches)?);
    Ok(())
}

fn run_verify_staged(args: VerifyStagedArgs) -> Result<()> {
    let state = StateStore::open(&args.state_dir)?;
    let build_id = args
        .build_id
        .or(state.last_staged_build_id()?)
        .ok_or_else(|| anyhow!("no staged build is available"))?;
    let stage_root = state.stage_root_for_build(build_id);
    let latest_staged = state.last_staged_build_id()?;
    let mut report = json!({
        "build_id": build_id,
        "stage_root": stage_root,
        "stage_root_exists": stage_root.is_dir(),
        "publish_db_exists": stage_root.join("index-publish.db").is_file(),
    });

    if latest_staged == Some(build_id) {
        let versions = state.load_versions_current()?;
        let packages = state.load_packages_current()?;
        report["tracked_manifest_count"] = json!(versions.len());
        report["tracked_package_count"] = json!(packages.len());
        report["missing_manifests"] = json!(
            versions
                .values()
                .filter(|version| !stage_root
                    .join(&version.published_manifest_relpath)
                    .is_file())
                .map(|version| version.published_manifest_relpath.clone())
                .collect::<Vec<_>>()
        );
        report["missing_package_files"] = json!(
            packages
                .values()
                .filter(|package| !package.version_data_relpath.is_empty())
                .filter(|package| !stage_root.join(&package.version_data_relpath).is_file())
                .map(|package| package.version_data_relpath.clone())
                .collect::<Vec<_>>()
        );
    }

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn run_verify_published(args: VerifyPublishedArgs) -> Result<()> {
    let state = StateStore::open(&args.state_dir)?;
    let published = state.load_published_files_current()?;
    let report = collect_publish_tree_drift(&args.out_dir, &published)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn infer_staged_index_version(stage_root: &Path) -> Result<CatalogFormat> {
    let package_root = stage_root.join("packages");
    if package_root.is_dir() {
        Ok(CatalogFormat::V2)
    } else {
        Ok(CatalogFormat::V1)
    }
}

fn resolve_sign_password(
    explicit_password: Option<&str>,
    password_env: Option<&str>,
) -> Result<Option<String>> {
    if let Some(password) = explicit_password {
        return Ok(Some(password.to_string()));
    }
    let Some(password_env) = password_env else {
        return Ok(None);
    };
    let password = std::env::var(password_env)
        .with_context(|| format!("failed to read signing password from env var {password_env}"))?;
    Ok(Some(password))
}

fn collect_publish_tree_drift(
    out_dir: &Path,
    published: &HashMap<String, PublishedFile>,
) -> Result<PublishTreeDrift> {
    let mut missing = Vec::new();
    let mut mismatched = Vec::new();
    for file in published.values() {
        let path = out_dir.join(&file.relpath);
        if !path.is_file() {
            missing.push(file.relpath.clone());
            continue;
        }
        let hash = sha256_bytes(
            &fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?,
        );
        if hash != file.sha256 {
            mismatched.push(file.relpath.clone());
        }
    }

    let tracked = published.keys().cloned().collect::<HashSet<_>>();
    let extra_files = if out_dir.is_dir() {
        WalkDir::new(out_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .filter_map(|entry| {
                let rel =
                    normalize_rel(&entry.path().strip_prefix(out_dir).ok()?.to_string_lossy());
                (!tracked.contains(&rel)).then_some(rel)
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    Ok(PublishTreeDrift {
        tracked_files: tracked.len(),
        missing_files: missing,
        mismatched_hash_files: mismatched,
        extra_files,
    })
}

fn prune_staging_dirs(
    staging_root: PathBuf,
    keep_last: usize,
    older_than: Option<SystemTime>,
) -> Result<()> {
    if !staging_root.is_dir() {
        return Ok(());
    }

    let mut dirs = fs::read_dir(&staging_root)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    dirs.sort();
    dirs.reverse();
    for dir in dirs.into_iter().skip(keep_last) {
        if let Some(cutoff) = older_than {
            let modified = fs::metadata(&dir)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if modified >= cutoff {
                continue;
            }
        }
        fs::remove_dir_all(&dir).with_context(|| format!("failed to remove {}", dir.display()))?;
    }
    Ok(())
}

fn parse_older_than(value: Option<&str>) -> Result<Option<SystemTime>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    ensure!(!value.is_empty(), "--older-than cannot be empty");
    let split_index = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    let (digits, unit) = value.split_at(split_index);
    ensure!(!digits.is_empty(), "invalid --older-than value: {value}");
    let amount = digits
        .parse::<u64>()
        .with_context(|| format!("invalid --older-than value: {value}"))?;
    let seconds = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => amount,
        "m" | "min" | "mins" | "minute" | "minutes" => amount.saturating_mul(60),
        "h" | "hr" | "hrs" | "hour" | "hours" => amount.saturating_mul(60 * 60),
        "d" | "day" | "days" => amount.saturating_mul(60 * 60 * 24),
        "w" | "week" | "weeks" => amount.saturating_mul(60 * 60 * 24 * 7),
        _ => bail!("unsupported --older-than unit in {value}; use s, m, h, d, or w"),
    };
    let duration = Duration::from_secs(seconds);
    Ok(Some(
        SystemTime::now()
            .checked_sub(duration)
            .unwrap_or(UNIX_EPOCH),
    ))
}

fn system_time_to_unix(value: SystemTime) -> i64 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn resolve_state_version<'a>(
    versions: &'a HashMap<String, StoredVersion>,
    version_dir: Option<&Path>,
    package_id: Option<&str>,
    version: Option<&str>,
) -> Result<&'a StoredVersion> {
    match (version_dir, package_id, version) {
        (Some(version_dir), None, None) => {
            let version_dir = normalize_rel(&version_dir.to_string_lossy());
            versions
                .get(version_dir.as_str())
                .ok_or_else(|| anyhow!("version {} not found", version_dir))
        }
        (None, Some(package_id), Some(version)) => {
            let matches = versions
                .values()
                .filter(|stored| {
                    stored.package_id == package_id && stored.package_version == version
                })
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [only] => Ok(*only),
                [] => bail!("package {} version {} not found", package_id, version),
                _ => bail!(
                    "multiple versions match package {} version {}; use --version-dir",
                    package_id,
                    version
                ),
            }
        }
        _ => bail!("either --version-dir or --package-id with --version is required"),
    }
}

fn compute_repo_target_snapshot(args: &RepoTargetArgs) -> Result<ComputedVersionSnapshot> {
    let repo_root = args
        .repo_dir
        .canonicalize()
        .with_context(|| format!("failed to resolve repo path {}", args.repo_dir.display()))?;
    let scan_root = scan_root(&repo_root);
    let progress = ProgressReporter::new();
    let current_files = scan_yaml_files(&repo_root, &scan_root, &progress, &Messages::new("en"))?;
    let spec = TargetSelectionSpec {
        version_dirs: args.version_dir.clone().into_iter().collect(),
        manifest_files: args.manifest_file.clone().into_iter().collect(),
        package_ids: Vec::new(),
        package_id: args.package_id.clone(),
        version: args.version.clone(),
    };
    validate_target_selection_spec(&spec)?;
    let targets =
        resolve_target_selection(&repo_root, &current_files, &HashMap::new(), Some(&spec))?;
    let version_dir = targets
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no target version directory resolved"))?;
    let current_version_abs = current_files
        .values()
        .map(|file| (file.version_dir.clone(), file.version_dir_abs.clone()))
        .collect::<HashMap<_, _>>();
    let abs = current_version_abs
        .get(version_dir.as_str())
        .ok_or_else(|| {
            anyhow!(
                "resolved version dir {} is not present in repo",
                version_dir
            )
        })?;
    Ok(compute_version_snapshot_with_warnings(&repo_root, abs, &version_dir)?.snapshot)
}

fn run_build_inner(
    args: &BuildArgs,
    state: &mut StateStore,
    build_id: i64,
    messages: Messages,
    options: BuildExecutionOptions<'_>,
) -> Result<()> {
    let progress = ProgressReporter::new();
    let index_package_name = args.index_version.package_file_name();

    let repo_root = args
        .repo_dir
        .canonicalize()
        .with_context(|| format!("failed to resolve repo path {}", args.repo_dir.display()))?;
    let scan_root = scan_root(&repo_root);

    info!("{}", messages.scanning_repository(&scan_root));

    let previous_files = state.load_files_current()?;
    let previous_versions = state.load_versions_current()?;
    let previous_packages = state.load_packages_current()?;
    let last_successful_unix = state.last_successful_build_epoch()?;
    let previous_stage_root = state
        .last_staged_build_id()?
        .map(|stage_build_id| state.stage_root_for_build(stage_build_id))
        .filter(|path| path.is_dir());
    let previous_stage_matches = previous_stage_root.is_some()
        && state.last_staged_index_version()? == Some(args.index_version)
        && state.last_staged_backend()? == Some(args.backend);
    // A stage rebuild is required whenever the selected backend/index-version pair changes,
    // because the staged artifact set is backend-specific.
    let full_stage_rebuild = args.force || !previous_stage_matches;

    let mut current_files = scan_yaml_files(&repo_root, &scan_root, &progress, &messages)?;
    fill_file_hashes(&mut current_files, &previous_files, &progress, &messages)?;
    let resolved_targets = resolve_target_selection(
        &repo_root,
        &current_files,
        &previous_versions,
        options.target_spec,
    )?;
    if options.remove_selected && !resolved_targets.is_empty() {
        current_files.retain(|_, file| !resolved_targets.contains(&file.version_dir));
    }

    let dirty_version_dirs = if resolved_targets.is_empty() {
        determine_dirty_version_dirs(&current_files, &previous_files)
    } else {
        resolved_targets.clone()
    };
    let current_version_dirs = current_files
        .values()
        .map(|file| file.version_dir.clone())
        .collect::<HashSet<_>>();
    let metadata_backfill_needed = current_version_dirs.iter().any(|version_dir| {
        previous_versions
            .get(version_dir.as_str())
            .is_some_and(|version| {
                version.index_projection_json.is_none() || version.installers_json.is_none()
            })
    });
    let version_dirs_to_refresh = if metadata_backfill_needed || full_stage_rebuild {
        dirty_version_dirs
            .union(&current_version_dirs)
            .cloned()
            .collect::<HashSet<_>>()
    } else {
        dirty_version_dirs.clone()
    };

    let current_version_abs = current_files
        .values()
        .map(|file| (file.version_dir.clone(), file.version_dir_abs.clone()))
        .collect::<HashMap<_, _>>();

    let dirty_existing_version_dirs = version_dirs_to_refresh
        .iter()
        .filter(|version_dir| current_version_dirs.contains(*version_dir))
        .cloned()
        .collect::<Vec<_>>();

    info!(
        "{}",
        messages.dirty_versions_detected(dirty_existing_version_dirs.len())
    );

    let mut computed_versions = compute_dirty_versions(
        &repo_root,
        &current_version_abs,
        &dirty_existing_version_dirs,
        &progress,
        &messages,
    )?;
    // ARP DisplayVersion rewriting can introduce synthetic manifest updates even when the source
    // files are unchanged, so it runs before semantic diffing.
    let arp_policy_changed_version_dirs = apply_arp_display_version_policy(
        &repo_root,
        &current_version_abs,
        &version_dirs_to_refresh,
        &previous_versions,
        &mut computed_versions,
        &progress,
        &messages,
        args.display_version_conflict_strategy,
    )?;
    let version_dirs_to_compare = version_dirs_to_refresh
        .union(&arp_policy_changed_version_dirs)
        .cloned()
        .collect::<HashSet<_>>();

    let (version_changes, semantic_changes, validation_requirements) =
        build_version_changes_and_validation_queue(
            &version_dirs_to_compare,
            &computed_versions,
            &previous_versions,
        )?;

    state.record_version_changes(build_id, &version_changes)?;
    if options.write_validation_queue_file {
        let validation_queue_path = state.validation_queue_path();
        write_validation_queue(validation_queue_path.clone(), &validation_requirements)?;
        info!(
            "{}",
            messages
                .validation_queue_written(validation_requirements.len(), &validation_queue_path)
        );
    }

    let semantic_version_changes = semantic_changes
        .iter()
        .filter(|change| !matches!(change, VersionSemanticChange::Noop))
        .count();
    let mut final_versions = previous_versions.clone();
    for version_dir in sorted_strings(version_dirs_to_compare.iter()) {
        if let Some(new_version) = computed_versions.get(version_dir.as_str()) {
            final_versions.insert(
                version_dir.clone(),
                stored_version_from_computed(new_version),
            );
        } else {
            final_versions.remove(version_dir.as_str());
        }
    }

    if options.dry_run {
        info!(
            "{}",
            json!({
                "build_id": build_id,
                "changed_versions": semantic_version_changes,
                "validation_queue_items": validation_requirements.len(),
                "touched_targets": dirty_version_dirs.len(),
                "remove_selected": options.remove_selected,
            })
        );
        return Ok(());
    }

    if semantic_version_changes == 0 && !full_stage_rebuild {
        info!("{}", messages.no_semantic_changes());
        let stage_root = state.stage_root_for_build(build_id);
        let previous_stage_root = previous_stage_root
            .as_ref()
            .ok_or_else(|| anyhow!("missing previous staged build root for no-op rebuild"))?;
        // A no-op build still gets its own staged directory so publish can be driven by build id,
        // but we can reuse the previous staged payload byte-for-byte.
        if stage_root.exists() {
            fs::remove_dir_all(&stage_root)
                .with_context(|| format!("failed to clear {}", stage_root.display()))?;
        }
        copy_tree(previous_stage_root, &stage_root)?;
        let finished_unix = unix_now()?;
        let final_files = current_files
            .values()
            .map(current_file_to_stored)
            .collect::<Vec<_>>();
        let final_versions_vec = final_versions.values().cloned().collect::<Vec<_>>();
        let final_packages_vec = previous_packages.values().cloned().collect::<Vec<_>>();
        state.replace_working_state(WorkingStateUpdate {
            build_id,
            finished_unix,
            files: &final_files,
            versions: &final_versions_vec,
            packages: &final_packages_vec,
            index_version: args.index_version,
            backend: args.backend,
            build_status: "staged_reused",
        })?;
        info!("{}", messages.build_staged(&stage_root, &args.state_dir));
        return Ok(());
    }

    info!(
        "{}",
        messages.staging_publish_tree(semantic_version_changes)
    );
    let stage_root = state.stage_root_for_build(build_id);
    if stage_root.exists() {
        fs::remove_dir_all(&stage_root)
            .with_context(|| format!("failed to clear {}", stage_root.display()))?;
    }
    fs::create_dir_all(&stage_root)
        .with_context(|| format!("failed to create {}", stage_root.display()))?;

    let mut adapter_remove_ops = Vec::<AdapterOperation>::new();
    let mut adapter_add_ops = Vec::<AdapterOperation>::new();
    let mut changed_manifest_relpaths = BTreeSet::<String>::new();
    let mut touched_packages = BTreeSet::<String>::new();

    let staging_units = if full_stage_rebuild {
        final_versions.len()
    } else {
        semantic_version_changes
    };
    let staging_progress = progress.bar(staging_units, messages.progress_staging_manifests());
    if full_stage_rebuild {
        for version_dir in sorted_strings(current_version_dirs.iter()) {
            let snapshot = computed_versions
                .get(version_dir.as_str())
                .ok_or_else(|| anyhow!("missing computed snapshot for {version_dir}"))?;
            stage_manifest(&stage_root, snapshot)?;
            changed_manifest_relpaths.insert(snapshot.published_manifest_relpath.clone());
            touched_packages.insert(snapshot.package_id.clone());
            if args.backend == BackendKind::Wingetutil {
                adapter_add_ops.push(AdapterOperation {
                    kind: "add".to_string(),
                    manifest_path: absolute_string(
                        &stage_root.join(&snapshot.published_manifest_relpath),
                    ),
                    relative_path: snapshot.published_manifest_relpath.clone(),
                });
            }
            ProgressReporter::inc(&staging_progress, 1);
        }
        touched_packages.extend(previous_packages.keys().cloned());
    } else {
        let previous_stage_root = previous_stage_root
            .as_ref()
            .ok_or_else(|| anyhow!("missing previous staged build root for incremental build"))?;
        for change in &semantic_changes {
            match change {
                VersionSemanticChange::Add(new_version) => {
                    stage_manifest(&stage_root, new_version)?;
                    adapter_add_ops.push(AdapterOperation {
                        kind: "add".to_string(),
                        manifest_path: absolute_string(
                            &stage_root.join(&new_version.published_manifest_relpath),
                        ),
                        relative_path: new_version.published_manifest_relpath.clone(),
                    });
                    changed_manifest_relpaths
                        .insert(new_version.published_manifest_relpath.clone());
                    touched_packages.insert(new_version.package_id.clone());
                    ProgressReporter::inc(&staging_progress, 1);
                }
                VersionSemanticChange::Update { old, new } => {
                    stage_manifest(&stage_root, new)?;
                    if args.backend == BackendKind::Wingetutil {
                        let old_abs = previous_stage_root.join(&old.published_manifest_relpath);
                        ensure!(
                            old_abs.is_file(),
                            "existing staged manifest is missing: {}",
                            old_abs.display()
                        );
                        adapter_remove_ops.push(AdapterOperation {
                            kind: "remove".to_string(),
                            manifest_path: absolute_string(&old_abs),
                            relative_path: old.published_manifest_relpath.clone(),
                        });
                    }
                    adapter_add_ops.push(AdapterOperation {
                        kind: "add".to_string(),
                        manifest_path: absolute_string(
                            &stage_root.join(&new.published_manifest_relpath),
                        ),
                        relative_path: new.published_manifest_relpath.clone(),
                    });
                    changed_manifest_relpaths.insert(new.published_manifest_relpath.clone());
                    touched_packages.insert(old.package_id.clone());
                    touched_packages.insert(new.package_id.clone());
                    ProgressReporter::inc(&staging_progress, 1);
                }
                VersionSemanticChange::Remove(old) => {
                    if args.backend == BackendKind::Wingetutil {
                        let old_abs = previous_stage_root.join(&old.published_manifest_relpath);
                        ensure!(
                            old_abs.is_file(),
                            "existing staged manifest is missing: {}",
                            old_abs.display()
                        );
                        adapter_remove_ops.push(AdapterOperation {
                            kind: "remove".to_string(),
                            manifest_path: absolute_string(&old_abs),
                            relative_path: old.published_manifest_relpath.clone(),
                        });
                    }
                    touched_packages.insert(old.package_id.clone());
                    ProgressReporter::inc(&staging_progress, 1);
                }
                VersionSemanticChange::Noop => {}
            }
        }
    }
    ProgressReporter::finish(staging_progress);

    let mut adapter_ops = adapter_remove_ops;
    adapter_ops.extend(adapter_add_ops);

    match args.backend {
        BackendKind::Wingetutil => {
            if full_stage_rebuild {
                let mutable_db_path = state.mutable_db_path_for_format(args.index_version);
                if mutable_db_path.exists() {
                    fs::remove_file(&mutable_db_path).with_context(|| {
                        format!("failed to remove {}", mutable_db_path.display())
                    })?;
                }
            }
            let candidate_db_path = stage_root.join(format!(
                "mutable-{}.db",
                args.index_version
                    .package_file_name()
                    .trim_end_matches(".msix")
            ));
            let publish_db_path = stage_root.join("index-publish.db");
            let (schema_major_version, schema_minor_version) =
                args.index_version.wingetutil_schema_version();
            let adapter_request = AdapterRequest {
                mutable_db_path: absolute_string(
                    &state.mutable_db_path_for_format(args.index_version),
                ),
                candidate_db_path: absolute_string(&candidate_db_path),
                publish_db_path: absolute_string(&publish_db_path),
                stage_root: absolute_string(&stage_root),
                package_update_tracking_base_time: last_successful_unix,
                schema_major_version,
                schema_minor_version,
                operations: adapter_ops,
            };

            info!("{}", messages.running_adapter(index_package_name));
            let adapter_progress =
                progress.spinner(messages.progress_running_adapter(index_package_name));
            run_adapter(&adapter_request, &stage_root)?;
            ProgressReporter::finish(adapter_progress);
            commit_mutable_db(
                state.mutable_db_path_for_format(args.index_version),
                &candidate_db_path,
            )?;
        }
        BackendKind::Rust => {
            info!("{}", messages.running_rust_backend(index_package_name));
            let backend_progress =
                progress.spinner(messages.progress_running_rust_backend(index_package_name));
            run_rust_backend(
                &stage_root,
                &final_versions,
                &previous_packages,
                &touched_packages,
                last_successful_unix,
                args.index_version,
            )?;
            ProgressReporter::finish(backend_progress);
        }
    }

    let publish_db_path = stage_root.join("index-publish.db");
    ensure!(
        publish_db_path.is_file(),
        "backend packaging did not produce {}",
        publish_db_path.display()
    );

    let staged_package_files = if args.index_version.uses_package_sidecars() {
        scan_staged_package_files(&stage_root)?
    } else {
        HashMap::new()
    };

    let mut package_changes = Vec::<BuildPackageChange>::new();
    let final_packages_map = if args.index_version.uses_package_sidecars() {
        build_final_packages(
            &final_versions,
            &previous_packages,
            &staged_package_files,
            &touched_packages,
            &validation_requirements,
            &mut package_changes,
        )?
    } else {
        HashMap::new()
    };
    state.record_package_changes(build_id, &package_changes)?;

    if !full_stage_rebuild {
        stage_unchanged_artifacts_from_previous(
            previous_stage_root
                .as_deref()
                .ok_or_else(|| anyhow!("missing previous staged build root"))?,
            &stage_root,
            &final_versions,
            &final_packages_map,
            &changed_manifest_relpaths,
            &touched_packages,
            args.index_version,
        )?;
    }

    let final_files = current_files
        .values()
        .map(current_file_to_stored)
        .collect::<Vec<_>>();
    let final_versions_vec = final_versions.values().cloned().collect::<Vec<_>>();
    let final_packages_vec = final_packages_map.values().cloned().collect::<Vec<_>>();

    let finished_unix = unix_now()?;
    state.replace_working_state(WorkingStateUpdate {
        build_id,
        finished_unix,
        files: &final_files,
        versions: &final_versions_vec,
        packages: &final_packages_vec,
        index_version: args.index_version,
        backend: args.backend,
        build_status: "staged",
    })?;
    info!("{}", messages.build_staged(&stage_root, &args.state_dir));

    Ok(())
}

fn collect_diff_summary(
    repo_dir: &Path,
    state: &StateStore,
    messages: &Messages,
    target_spec: Option<&TargetSelectionSpec>,
) -> Result<DiffSummary> {
    let progress = ProgressReporter::new();
    let repo_root = repo_dir
        .canonicalize()
        .with_context(|| format!("failed to resolve repo path {}", repo_dir.display()))?;
    let scan_root = scan_root(&repo_root);
    info!("{}", messages.scanning_repository(&scan_root));

    let previous_files = state.load_files_current()?;
    let previous_versions = state.load_versions_current()?;
    let mut current_files = scan_yaml_files(&repo_root, &scan_root, &progress, messages)?;
    fill_file_hashes(&mut current_files, &previous_files, &progress, messages)?;
    let resolved_targets =
        resolve_target_selection(&repo_root, &current_files, &previous_versions, target_spec)?;

    let current_version_abs = current_files
        .values()
        .map(|file| (file.version_dir.clone(), file.version_dir_abs.clone()))
        .collect::<HashMap<_, _>>();
    let dirty_version_dirs = if resolved_targets.is_empty() {
        determine_dirty_version_dirs(&current_files, &previous_files)
    } else {
        resolved_targets
    };
    let dirty_existing_version_dirs = dirty_version_dirs
        .iter()
        .filter(|version_dir| current_version_abs.contains_key(*version_dir))
        .cloned()
        .collect::<Vec<_>>();
    let mut computed_versions = compute_dirty_versions(
        &repo_root,
        &current_version_abs,
        &dirty_existing_version_dirs,
        &progress,
        messages,
    )?;
    let arp_policy_changed_version_dirs = apply_arp_display_version_policy(
        &repo_root,
        &current_version_abs,
        &dirty_version_dirs,
        &previous_versions,
        &mut computed_versions,
        &progress,
        messages,
        DisplayVersionConflictStrategy::Latest,
    )?;
    let version_dirs_to_compare = dirty_version_dirs
        .union(&arp_policy_changed_version_dirs)
        .cloned()
        .collect::<HashSet<_>>();
    let (version_changes, _, validation_requirements) = build_version_changes_and_validation_queue(
        &version_dirs_to_compare,
        &computed_versions,
        &previous_versions,
    )?;

    Ok(DiffSummary {
        added_versions: version_changes
            .iter()
            .filter(|change| change.change_kind == "add")
            .count(),
        updated_versions: version_changes
            .iter()
            .filter(|change| change.change_kind == "update")
            .count(),
        removed_versions: version_changes
            .iter()
            .filter(|change| change.change_kind == "remove")
            .count(),
        installer_revalidations: validation_requirements.len(),
    })
}

fn filter_build_records(
    builds: Vec<BuildRecord>,
    filters: &[BuildRecordStatusFilter],
) -> Vec<BuildRecord> {
    if filters.is_empty() {
        return builds;
    }

    builds
        .into_iter()
        .filter(|build| {
            filters.iter().any(|filter| match filter {
                BuildRecordStatusFilter::Running => build.status == "running",
                BuildRecordStatusFilter::Staged => {
                    build.status == "staged" || build.status == "staged_reused"
                }
                BuildRecordStatusFilter::Published => build.status == "published",
                BuildRecordStatusFilter::Failed => build.status == "failed",
            })
        })
        .collect()
}

fn count_validation_queue_items(path: &Path) -> Result<usize> {
    if !path.is_file() {
        return Ok(0);
    }

    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let items: Vec<ValidationRequirement> =
        serde_json::from_slice(&data).context("failed to parse validation queue JSON")?;
    Ok(items.len())
}

fn validate_target_selection_spec(spec: &TargetSelectionSpec) -> Result<()> {
    let mut modes = 0usize;
    if !spec.version_dirs.is_empty() {
        modes += 1;
    }
    if !spec.manifest_files.is_empty() {
        modes += 1;
    }
    if !spec.package_ids.is_empty() {
        modes += 1;
    }
    if spec.package_id.is_some() || spec.version.is_some() {
        modes += 1;
    }

    ensure!(
        modes == 1,
        "exactly one target mode is required: --version-dir, --manifest-file, --package-id, or --package-id with --version"
    );
    if !spec.package_ids.is_empty() && (spec.package_id.is_some() || spec.version.is_some()) {
        bail!("repeated --package-id cannot be combined with --package-id and --version selection");
    }
    if spec.package_id.is_some() ^ spec.version.is_some() {
        bail!("--package-id and --version must be used together");
    }
    Ok(())
}

fn resolve_target_selection(
    repo_root: &Path,
    current_files: &HashMap<String, CurrentFileScan>,
    previous_versions: &HashMap<String, StoredVersion>,
    target_spec: Option<&TargetSelectionSpec>,
) -> Result<HashSet<String>> {
    let Some(target_spec) = target_spec else {
        return Ok(HashSet::new());
    };

    let current_version_abs = current_files
        .values()
        .map(|file| (file.version_dir.clone(), file.version_dir_abs.clone()))
        .collect::<HashMap<_, _>>();

    let mut selected = HashSet::new();

    if !target_spec.version_dirs.is_empty() {
        selected.extend(
            target_spec
                .version_dirs
                .iter()
                .map(|path| normalize_version_dir_arg(repo_root, path))
                .collect::<Result<HashSet<_>>>()?,
        );
    }

    if !target_spec.manifest_files.is_empty() {
        selected.extend(
            target_spec
                .manifest_files
                .iter()
                .map(|path| normalize_manifest_file_arg(repo_root, path))
                .collect::<Result<HashSet<_>>>()?,
        );
    }

    if !target_spec.package_ids.is_empty() {
        let package_ids = target_spec
            .package_ids
            .iter()
            .map(|value| value.as_str())
            .collect::<HashSet<_>>();
        let mut matches = previous_versions
            .values()
            .filter(|stored| package_ids.contains(stored.package_id.as_str()))
            .map(|stored| stored.version_dir.clone())
            .collect::<BTreeSet<_>>();

        let unique_version_dirs = current_version_abs
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let computed = unique_version_dirs
            .par_iter()
            .filter_map(|version_dir| {
                let abs = current_version_abs.get(version_dir)?;
                let result =
                    compute_version_snapshot_with_warnings(repo_root, abs, version_dir).ok()?;
                package_ids
                    .contains(result.snapshot.package_id.as_str())
                    .then_some(version_dir.clone())
            })
            .collect::<Vec<_>>();
        matches.extend(computed);
        selected.extend(matches);
    }

    if target_spec.package_id.is_none() && target_spec.version.is_none() {
        return Ok(selected);
    }

    let package_id = target_spec
        .package_id
        .as_deref()
        .ok_or_else(|| anyhow!("missing --package-id"))?;
    let version = target_spec
        .version
        .as_deref()
        .ok_or_else(|| anyhow!("missing --version"))?;

    let mut matches = previous_versions
        .values()
        .filter(|stored| stored.package_id == package_id && stored.package_version == version)
        .map(|stored| stored.version_dir.clone())
        .collect::<BTreeSet<_>>();
    if matches.len() == 1 {
        selected.extend(matches);
        return Ok(selected);
    }
    if matches.len() > 1 {
        bail!(
            "multiple state entries match package {} version {}; use --version-dir instead",
            package_id,
            version
        );
    }

    let unique_version_dirs = current_version_abs
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut computed = unique_version_dirs
        .par_iter()
        .filter_map(|version_dir| {
            let abs = current_version_abs.get(version_dir)?;
            let result =
                compute_version_snapshot_with_warnings(repo_root, abs, version_dir).ok()?;
            if result.snapshot.package_id == package_id
                && result.snapshot.package_version == version
            {
                Some(version_dir.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    computed.sort();
    matches.extend(computed);

    if matches.is_empty() {
        bail!(
            "no version directory found for package {} version {}",
            package_id,
            version
        );
    }
    if matches.len() > 1 {
        bail!(
            "multiple repo entries match package {} version {}; use --version-dir instead",
            package_id,
            version
        );
    }

    selected.extend(matches);
    Ok(selected)
}

fn normalize_version_dir_arg(repo_root: &Path, path: &Path) -> Result<String> {
    let path = if path.is_absolute() {
        path.strip_prefix(repo_root)
            .with_context(|| format!("{} is not under {}", path.display(), repo_root.display()))?
            .to_path_buf()
    } else {
        path.to_path_buf()
    };
    Ok(normalize_rel(&path.to_string_lossy()))
}

fn normalize_manifest_file_arg(repo_root: &Path, path: &Path) -> Result<String> {
    let file_path = if path.is_absolute() {
        path.strip_prefix(repo_root)
            .with_context(|| format!("{} is not under {}", path.display(), repo_root.display()))?
            .to_path_buf()
    } else {
        path.to_path_buf()
    };
    let version_dir = file_path.parent().ok_or_else(|| {
        anyhow!(
            "manifest file {} has no parent version directory",
            path.display()
        )
    })?;
    Ok(normalize_rel(&version_dir.to_string_lossy()))
}

fn scan_yaml_files(
    repo_root: &Path,
    scan_root: &Path,
    progress: &ProgressReporter,
    messages: &Messages,
) -> Result<HashMap<String, CurrentFileScan>> {
    let mut result = HashMap::new();
    let entries = WalkDir::new(scan_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("yaml"))
        .collect::<Vec<_>>();
    let scan_progress = progress.bar(entries.len(), messages.progress_scanning_files());

    for path in entries {
        let metadata =
            fs::metadata(&path).with_context(|| format!("failed to stat {}", path.display()))?;
        let rel = normalize_rel(&path.strip_prefix(repo_root)?.to_string_lossy());
        let version_dir_abs = path
            .parent()
            .ok_or_else(|| anyhow!("manifest file {} has no parent directory", path.display()))?
            .to_path_buf();
        let version_dir =
            normalize_rel(&version_dir_abs.strip_prefix(repo_root)?.to_string_lossy());

        result.insert(
            rel.clone(),
            CurrentFileScan {
                path: rel,
                version_dir,
                abs_path: path.to_path_buf(),
                version_dir_abs,
                size: metadata.len(),
                mtime_ns: modified_to_unix_nanos(&metadata.modified()?)?,
                raw_sha256: Vec::new(),
            },
        );

        ProgressReporter::inc(&scan_progress, 1);
    }

    ProgressReporter::finish(scan_progress);

    Ok(result)
}

fn fill_file_hashes(
    current_files: &mut HashMap<String, CurrentFileScan>,
    previous_files: &HashMap<String, StoredFile>,
    progress: &ProgressReporter,
    messages: &Messages,
) -> Result<()> {
    // Partition files into those that need hashing and those that can reuse previous hashes
    let mut to_hash: Vec<(String, PathBuf)> = Vec::with_capacity(current_files.len());
    let mut reusable: HashMap<String, Vec<u8>> = HashMap::with_capacity(current_files.len());

    for (path, file) in current_files.iter() {
        if let Some(previous) = previous_files.get(path) {
            if previous.size == file.size && previous.mtime_ns == file.mtime_ns {
                reusable.insert(path.clone(), previous.raw_sha256.clone());
            } else {
                to_hash.push((path.clone(), file.abs_path.clone()));
            }
        } else {
            to_hash.push((path.clone(), file.abs_path.clone()));
        }
    }

    // Hash files in parallel
    let hash_progress = progress.bar(to_hash.len(), messages.progress_hashing_files());
    let hashed: HashMap<String, Vec<u8>> = to_hash
        .into_par_iter()
        .map(|(path, abs_path)| {
            let bytes = fs::read(&abs_path)
                .with_context(|| format!("failed to read manifest {}", abs_path.display()))?;
            ProgressReporter::inc(&hash_progress, 1);
            Ok::<_, anyhow::Error>((path, sha256_bytes(&bytes)))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .collect();

    // Merge the results into current_files
    for (path, file) in current_files.iter_mut() {
        file.raw_sha256 = reusable
            .remove(path)
            .or_else(|| hashed.get(path).cloned())
            .ok_or_else(|| anyhow!("missing hash for {}", file.abs_path.display()))?;
    }

    ProgressReporter::finish(hash_progress);
    Ok(())
}

fn determine_dirty_version_dirs(
    current_files: &HashMap<String, CurrentFileScan>,
    previous_files: &HashMap<String, StoredFile>,
) -> HashSet<String> {
    let mut dirty = HashSet::with_capacity(current_files.len().max(previous_files.len()) / 2);

    // Check for modified or new files using entry API for efficiency
    for (path, current) in current_files {
        if let Some(previous) = previous_files.get(path) {
            if previous.raw_sha256 != current.raw_sha256 {
                dirty.insert(current.version_dir.clone());
            }
        } else {
            dirty.insert(current.version_dir.clone());
        }
    }

    // Check for removed files
    for (path, previous) in previous_files {
        if !current_files.contains_key(path) {
            dirty.insert(previous.version_dir.clone());
        }
    }

    dirty
}

fn compute_dirty_versions(
    repo_root: &Path,
    current_version_abs: &HashMap<String, PathBuf>,
    dirty_existing_version_dirs: &[String],
    progress: &ProgressReporter,
    messages: &Messages,
) -> Result<HashMap<String, ComputedVersionSnapshot>> {
    let version_progress = progress.bar(
        dirty_existing_version_dirs.len(),
        messages.progress_computing_versions(),
    );
    let results = dirty_existing_version_dirs
        .par_iter()
        .map(|version_dir| {
            let abs = current_version_abs
                .get(version_dir)
                .ok_or_else(|| anyhow!("missing absolute path for version dir {version_dir}"))?;
            let result = compute_version_snapshot_with_warnings(repo_root, abs, version_dir)?;
            for warning in &result.warnings {
                match warning {
                    ManifestWarning::NumericPackageVersion {
                        manifest_path,
                        package_version,
                    } => progress.warn(
                        messages.warning_numeric_package_version(manifest_path, package_version),
                    ),
                }
            }
            ProgressReporter::inc(&version_progress, 1);
            Ok::<_, anyhow::Error>((version_dir.clone(), result.snapshot))
        })
        .collect::<Vec<_>>();

    let mut computed = HashMap::new();
    for item in results {
        let (version_dir, snapshot) = item?;
        computed.insert(version_dir, snapshot);
    }
    ProgressReporter::finish(version_progress);
    Ok(computed)
}

#[allow(clippy::too_many_arguments)]
fn apply_arp_display_version_policy(
    repo_root: &Path,
    current_version_abs: &HashMap<String, PathBuf>,
    dirty_version_dirs: &HashSet<String>,
    previous_versions: &HashMap<String, StoredVersion>,
    computed_versions: &mut HashMap<String, ComputedVersionSnapshot>,
    progress: &ProgressReporter,
    messages: &Messages,
    strategy: DisplayVersionConflictStrategy,
) -> Result<HashSet<String>> {
    let touched_packages =
        determine_touched_packages(dirty_version_dirs, previous_versions, computed_versions);
    if touched_packages.is_empty() {
        return Ok(HashSet::new());
    }

    let current_versions_by_package = build_current_versions_by_package(
        current_version_abs,
        previous_versions,
        computed_versions,
    );

    let missing_version_dirs = touched_packages
        .iter()
        .flat_map(|package_id| {
            current_versions_by_package
                .get(package_id.as_str())
                .into_iter()
                .flatten()
                .filter(|version_dir| !computed_versions.contains_key(version_dir.as_str()))
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect::<HashSet<_>>();

    if !missing_version_dirs.is_empty() {
        let additional_versions = compute_dirty_versions(
            repo_root,
            current_version_abs,
            &sorted_strings(missing_version_dirs.iter()),
            progress,
            messages,
        )?;
        computed_versions.extend(additional_versions);
    }

    let mut changed_version_dirs = HashSet::new();

    for package_id in touched_packages {
        let Some(version_dirs) = current_versions_by_package.get(package_id.as_str()) else {
            continue;
        };

        let mut display_versions_by_version = HashMap::<String, BTreeSet<String>>::new();
        let mut contenders_by_display_version = HashMap::<String, Vec<String>>::new();

        for version_dir in version_dirs {
            let Some(snapshot) = computed_versions.get(version_dir.as_str()) else {
                continue;
            };

            let display_versions =
                extract_display_versions_from_manifest_bytes(&snapshot.published_manifest_bytes)?;
            for display_version in &display_versions {
                contenders_by_display_version
                    .entry(display_version.clone())
                    .or_default()
                    .push(version_dir.clone());
            }
            display_versions_by_version.insert(version_dir.clone(), display_versions);
        }

        let mut retained_display_versions = display_versions_by_version.clone();
        for (display_version, contenders) in contenders_by_display_version {
            if contenders.len() <= 1 {
                continue;
            }

            match strategy {
                DisplayVersionConflictStrategy::Latest | DisplayVersionConflictStrategy::Oldest => {
                    let winner = select_display_version_conflict_winner(
                        computed_versions,
                        &contenders,
                        strategy,
                    )?
                    .ok_or_else(|| anyhow!("display version contenders unexpectedly empty"))?;
                    let stripped_versions = contenders
                        .iter()
                        .filter(|version_dir| *version_dir != &winner)
                        .map(|version_dir| {
                            describe_snapshot_version(computed_versions, version_dir)
                        })
                        .collect::<Vec<_>>();

                    if !stripped_versions.is_empty() {
                        let retained_version =
                            describe_snapshot_version(computed_versions, &winner);
                        progress.warn(messages.warning_display_version_conflict(
                            package_id.as_str(),
                            &display_version,
                            &retained_version,
                            &stripped_versions.join(", "),
                        ));
                    }

                    for (version_dir, retained) in retained_display_versions.iter_mut() {
                        if version_dir != &winner {
                            retained.remove(&display_version);
                        }
                    }
                }
                DisplayVersionConflictStrategy::StripAll => {
                    progress.warn(format!(
                        "Warning: package {} has conflicting ARP DisplayVersion \"{}\"; removing it from {}.",
                        package_id,
                        display_version,
                        contenders
                            .iter()
                            .map(|version_dir| describe_snapshot_version(computed_versions, version_dir))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    for retained in retained_display_versions.values_mut() {
                        retained.remove(&display_version);
                    }
                }
                DisplayVersionConflictStrategy::Error => {
                    bail!(
                        "package {package_id} has conflicting ARP DisplayVersion \"{display_version}\" across versions: {}",
                        contenders
                            .iter()
                            .map(|version_dir| describe_snapshot_version(
                                computed_versions,
                                version_dir
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        }

        for version_dir in version_dirs {
            let Some(current_snapshot) = computed_versions.get(version_dir.as_str()).cloned()
            else {
                continue;
            };

            let desired_display_versions = retained_display_versions
                .get(version_dir.as_str())
                .cloned()
                .unwrap_or_default();
            let adjusted_snapshot = retain_display_versions_in_snapshot(
                repo_root,
                &current_snapshot,
                &desired_display_versions,
            )?;

            if previous_versions
                .get(version_dir.as_str())
                .is_none_or(|previous| stored_version_differs(previous, &adjusted_snapshot))
            {
                changed_version_dirs.insert(version_dir.clone());
            }

            computed_versions.insert(version_dir.clone(), adjusted_snapshot);
        }
    }

    Ok(changed_version_dirs)
}

fn select_display_version_conflict_winner(
    computed_versions: &HashMap<String, ComputedVersionSnapshot>,
    contenders: &[String],
    strategy: DisplayVersionConflictStrategy,
) -> Result<Option<String>> {
    let selection = match strategy {
        DisplayVersionConflictStrategy::Latest => contenders
            .iter()
            .max_by(|left, right| compare_snapshot_package_versions(computed_versions, left, right))
            .cloned(),
        DisplayVersionConflictStrategy::Oldest => contenders
            .iter()
            .min_by(|left, right| compare_snapshot_package_versions(computed_versions, left, right))
            .cloned(),
        DisplayVersionConflictStrategy::StripAll => None,
        DisplayVersionConflictStrategy::Error => unreachable!(),
    };

    Ok(selection)
}

fn describe_snapshot_version(
    computed_versions: &HashMap<String, ComputedVersionSnapshot>,
    version_dir: &str,
) -> String {
    let snapshot = computed_versions
        .get(version_dir)
        .expect("missing snapshot for version description");
    if snapshot.channel.is_empty() {
        snapshot.package_version.clone()
    } else {
        format!("{} [{}]", snapshot.package_version, snapshot.channel)
    }
}

fn determine_touched_packages(
    dirty_version_dirs: &HashSet<String>,
    previous_versions: &HashMap<String, StoredVersion>,
    computed_versions: &HashMap<String, ComputedVersionSnapshot>,
) -> BTreeSet<String> {
    let mut touched_packages = BTreeSet::new();

    for version_dir in dirty_version_dirs {
        if let Some(snapshot) = computed_versions.get(version_dir.as_str()) {
            touched_packages.insert(snapshot.package_id.clone());
        } else if let Some(previous) = previous_versions.get(version_dir.as_str()) {
            touched_packages.insert(previous.package_id.clone());
        }
    }

    touched_packages
}

fn build_current_versions_by_package(
    current_version_abs: &HashMap<String, PathBuf>,
    previous_versions: &HashMap<String, StoredVersion>,
    computed_versions: &HashMap<String, ComputedVersionSnapshot>,
) -> HashMap<String, Vec<String>> {
    let mut result = HashMap::<String, Vec<String>>::new();

    for version_dir in current_version_abs.keys() {
        let package_id = computed_versions
            .get(version_dir.as_str())
            .map(|snapshot| snapshot.package_id.as_str())
            .or_else(|| {
                previous_versions
                    .get(version_dir.as_str())
                    .map(|snapshot| snapshot.package_id.as_str())
            });

        let Some(package_id) = package_id else {
            continue;
        };

        result
            .entry(package_id.to_string())
            .or_default()
            .push(version_dir.clone());
    }

    for version_dirs in result.values_mut() {
        version_dirs.sort();
    }

    result
}

fn compare_snapshot_package_versions(
    computed_versions: &HashMap<String, ComputedVersionSnapshot>,
    left_version_dir: &str,
    right_version_dir: &str,
) -> std::cmp::Ordering {
    let left = computed_versions
        .get(left_version_dir)
        .expect("missing left snapshot");
    let right = computed_versions
        .get(right_version_dir)
        .expect("missing right snapshot");

    compare_versions(&left.package_version, &right.package_version)
        .then_with(|| left.channel.cmp(&right.channel))
        .then_with(|| left_version_dir.cmp(right_version_dir))
}

fn build_version_changes_and_validation_queue(
    version_dirs_to_compare: &HashSet<String>,
    computed_versions: &HashMap<String, ComputedVersionSnapshot>,
    previous_versions: &HashMap<String, StoredVersion>,
) -> Result<(
    Vec<BuildVersionChange>,
    Vec<VersionSemanticChange>,
    Vec<ValidationRequirement>,
)> {
    let mut version_changes = Vec::<BuildVersionChange>::new();
    let mut semantic_changes = Vec::<VersionSemanticChange>::new();
    let mut validation_requirements = Vec::<ValidationRequirement>::new();

    for version_dir in sorted_strings(version_dirs_to_compare.iter()) {
        let old = previous_versions.get(version_dir.as_str());
        let new = computed_versions.get(version_dir.as_str());

        match (old, new) {
            (None, Some(new_version)) => {
                version_changes.push(BuildVersionChange {
                    version_dir: version_dir.clone(),
                    package_id: new_version.package_id.clone(),
                    change_kind: "add".to_string(),
                    content_changed: true,
                    installer_changed: !new_version.installers.is_empty(),
                    old_content_sha256: None,
                    new_content_sha256: Some(new_version.version_content_sha256.clone()),
                });
                semantic_changes.push(VersionSemanticChange::Add(Box::new(new_version.clone())));
                validation_requirements.extend(new_version.installers.iter().cloned().map(
                    |installer| ValidationRequirement {
                        package_id: new_version.package_id.clone(),
                        package_version: new_version.package_version.clone(),
                        channel: new_version.channel.clone(),
                        installer,
                        reason: "added".to_string(),
                    },
                ));
            }
            (Some(old_version), None) => {
                version_changes.push(BuildVersionChange {
                    version_dir: version_dir.clone(),
                    package_id: old_version.package_id.clone(),
                    change_kind: "remove".to_string(),
                    content_changed: true,
                    installer_changed: false,
                    old_content_sha256: Some(old_version.version_content_sha256.clone()),
                    new_content_sha256: None,
                });
                semantic_changes.push(VersionSemanticChange::Remove(Box::new(old_version.clone())));
            }
            (Some(old_version), Some(new_version)) => {
                let content_changed = stored_version_differs(old_version, new_version);
                let previous_installers =
                    parse_installer_records_json(old_version.installers_json.as_deref())?;
                let added_installers =
                    added_installers(&previous_installers, &new_version.installers);
                let installer_changed = !added_installers.is_empty()
                    || previous_installers.len() != new_version.installers.len();

                if content_changed {
                    version_changes.push(BuildVersionChange {
                        version_dir: version_dir.clone(),
                        package_id: new_version.package_id.clone(),
                        change_kind: "update".to_string(),
                        content_changed: true,
                        installer_changed,
                        old_content_sha256: Some(old_version.version_content_sha256.clone()),
                        new_content_sha256: Some(new_version.version_content_sha256.clone()),
                    });
                    semantic_changes.push(VersionSemanticChange::Update {
                        old: Box::new(old_version.clone()),
                        new: Box::new(new_version.clone()),
                    });
                    validation_requirements.extend(added_installers.into_iter().map(|installer| {
                        ValidationRequirement {
                            package_id: new_version.package_id.clone(),
                            package_version: new_version.package_version.clone(),
                            channel: new_version.channel.clone(),
                            installer,
                            reason: "installer-changed".to_string(),
                        }
                    }));
                } else {
                    version_changes.push(BuildVersionChange {
                        version_dir: version_dir.clone(),
                        package_id: old_version.package_id.clone(),
                        change_kind: "noop".to_string(),
                        content_changed: false,
                        installer_changed,
                        old_content_sha256: Some(old_version.version_content_sha256.clone()),
                        new_content_sha256: Some(new_version.version_content_sha256.clone()),
                    });
                    semantic_changes.push(VersionSemanticChange::Noop);
                }
            }
            (None, None) => {}
        }
    }

    validation_requirements.sort_by(|left, right| {
        left.package_id
            .cmp(&right.package_id)
            .then_with(|| left.package_version.cmp(&right.package_version))
            .then_with(|| left.channel.cmp(&right.channel))
            .then_with(|| {
                left.installer
                    .installer_sha256
                    .cmp(&right.installer.installer_sha256)
            })
    });

    Ok((version_changes, semantic_changes, validation_requirements))
}

fn stored_version_differs(previous: &StoredVersion, current: &ComputedVersionSnapshot) -> bool {
    previous.version_content_sha256 != current.version_content_sha256
        || previous.published_manifest_sha256 != current.published_manifest_sha256
        || previous.published_manifest_relpath != current.published_manifest_relpath
        || previous.package_id != current.package_id
        || previous.package_version != current.package_version
        || previous.channel != current.channel
}

fn stage_manifest(stage_root: &Path, snapshot: &ComputedVersionSnapshot) -> Result<()> {
    let abs_path = stage_root.join(&snapshot.published_manifest_relpath);
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&abs_path, &snapshot.published_manifest_bytes)
        .with_context(|| format!("failed to write {}", abs_path.display()))?;
    Ok(())
}

fn scan_staged_package_files(stage_root: &Path) -> Result<HashMap<String, StagedPackageFile>> {
    let packages_root = stage_root.join("packages");
    let mut result = HashMap::new();

    if !packages_root.is_dir() {
        return Ok(result);
    }

    for entry in WalkDir::new(&packages_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) != Some("versionData.mszyml") {
            continue;
        }

        let rel = normalize_rel(&path.strip_prefix(stage_root)?.to_string_lossy());
        let sha256 = sha256_bytes(
            &fs::read(path).with_context(|| format!("failed to read {}", path.display()))?,
        );
        let mut components = path
            .strip_prefix(&packages_root)?
            .components()
            .map(|component| component.as_os_str().to_string_lossy().to_string());
        let package_id = components
            .next()
            .ok_or_else(|| anyhow!("invalid staged package path {}", path.display()))?;

        result.insert(
            package_id.clone(),
            StagedPackageFile {
                relpath: rel,
                sha256,
            },
        );
    }

    Ok(result)
}

fn build_final_packages(
    final_versions: &HashMap<String, StoredVersion>,
    previous_packages: &HashMap<String, StoredPackage>,
    staged_package_files: &HashMap<String, StagedPackageFile>,
    touched_packages: &BTreeSet<String>,
    validation_requirements: &[ValidationRequirement],
    package_changes: &mut Vec<BuildPackageChange>,
) -> Result<HashMap<String, StoredPackage>> {
    let mut result = previous_packages.clone();
    let mut versions_by_package = BTreeMap::<String, usize>::new();
    for version in final_versions.values() {
        *versions_by_package
            .entry(version.package_id.clone())
            .or_insert(0) += 1;
    }

    for package_id in touched_packages {
        let version_count = versions_by_package.get(package_id).copied().unwrap_or(0);
        let old = previous_packages.get(package_id);
        let installer_revalidate = validation_requirements
            .iter()
            .any(|item| item.package_id == *package_id);

        if version_count == 0 {
            if let Some(old_pkg) = old {
                package_changes.push(BuildPackageChange {
                    package_id: package_id.clone(),
                    change_kind: "remove".to_string(),
                    publish_changed: true,
                    installer_revalidate,
                    old_publish_sha256: Some(old_pkg.package_publish_sha256.clone()),
                    new_publish_sha256: None,
                });
                result.remove(package_id);
            }
            continue;
        }

        let staged = staged_package_files.get(package_id).ok_or_else(|| {
            anyhow!(
                "backend packaging did not emit versionData.mszyml for changed package {package_id}"
            )
        })?;

        let new_pkg = StoredPackage {
            package_id: package_id.clone(),
            version_count,
            version_data_relpath: staged.relpath.clone(),
            package_publish_sha256: staged.sha256.clone(),
        };

        package_changes.push(BuildPackageChange {
            package_id: package_id.clone(),
            change_kind: if old.is_some() {
                "update".to_string()
            } else {
                "add".to_string()
            },
            publish_changed: true,
            installer_revalidate,
            old_publish_sha256: old.map(|pkg| pkg.package_publish_sha256.clone()),
            new_publish_sha256: Some(new_pkg.package_publish_sha256.clone()),
        });
        result.insert(package_id.clone(), new_pkg);
    }

    Ok(result)
}

fn stage_unchanged_artifacts_from_previous(
    previous_stage_root: &Path,
    new_stage_root: &Path,
    final_versions: &HashMap<String, StoredVersion>,
    final_packages: &HashMap<String, StoredPackage>,
    changed_manifest_relpaths: &BTreeSet<String>,
    touched_packages: &BTreeSet<String>,
    index_version: CatalogFormat,
) -> Result<()> {
    for version in final_versions.values() {
        if changed_manifest_relpaths.contains(&version.published_manifest_relpath) {
            continue;
        }
        copy_selected_file(
            previous_stage_root,
            new_stage_root,
            &version.published_manifest_relpath,
        )?;
    }

    if index_version.uses_package_sidecars() {
        for package in final_packages.values() {
            if touched_packages.contains(&package.package_id) {
                continue;
            }
            copy_selected_file(
                previous_stage_root,
                new_stage_root,
                &package.version_data_relpath,
            )?;
        }
    }
    Ok(())
}

fn commit_staged_output_tree(
    stage_root: &Path,
    out_root: &Path,
    previous_published_files: &HashMap<String, PublishedFile>,
    final_versions: &HashMap<String, StoredVersion>,
    final_packages: &HashMap<String, StoredPackage>,
    catalog_package_name: &str,
) -> Result<()> {
    fs::create_dir_all(out_root)
        .with_context(|| format!("failed to create {}", out_root.display()))?;

    let final_published_relpaths =
        build_final_published_relpaths(final_versions, final_packages, catalog_package_name);

    for relpath in &final_published_relpaths {
        copy_selected_file(stage_root, out_root, relpath)?;
    }

    for relpath in previous_published_files.keys() {
        if final_published_relpaths.contains(relpath) {
            continue;
        }
        let target = out_root.join(relpath);
        if target.is_file() {
            fs::remove_file(&target)
                .with_context(|| format!("failed to delete {}", target.display()))?;
        }
    }

    Ok(())
}

fn build_final_published_relpaths(
    versions: &HashMap<String, StoredVersion>,
    packages: &HashMap<String, StoredPackage>,
    catalog_package_name: &str,
) -> HashSet<String> {
    let mut relpaths = HashSet::new();
    relpaths.insert(catalog_package_name.to_string());
    relpaths.extend(
        versions
            .values()
            .map(|version| version.published_manifest_relpath.clone()),
    );
    relpaths.extend(
        packages
            .values()
            .filter(|package| !package.version_data_relpath.is_empty())
            .map(|package| package.version_data_relpath.clone()),
    );
    relpaths
}

fn copy_selected_file(source_root: &Path, target_root: &Path, relpath: &str) -> Result<()> {
    let source = source_root.join(relpath);
    let target = target_root.join(relpath);
    ensure!(
        source.is_file(),
        "required file is missing: {}",
        source.display()
    );

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(&source, &target).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            target.display()
        )
    })?;
    Ok(())
}

fn copy_tree(source_root: &Path, target_root: &Path) -> Result<()> {
    fs::create_dir_all(target_root)
        .with_context(|| format!("failed to create {}", target_root.display()))?;

    for entry in WalkDir::new(source_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        let rel = path
            .strip_prefix(source_root)
            .with_context(|| format!("failed to relativize {}", path.display()))?;
        if rel.as_os_str().is_empty() {
            continue;
        }

        let target = target_root.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("failed to create {}", target.display()))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::copy(path, &target).with_context(|| {
                format!("failed to copy {} to {}", path.display(), target.display())
            })?;
        }
    }

    Ok(())
}

fn commit_mutable_db(final_path: PathBuf, candidate_path: &Path) -> Result<()> {
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let temp_path = final_path.with_extension("tmp");
    fs::copy(candidate_path, &temp_path).with_context(|| {
        format!(
            "failed to copy candidate mutable db {} to {}",
            candidate_path.display(),
            temp_path.display()
        )
    })?;

    if final_path.exists() {
        fs::remove_file(&final_path)
            .with_context(|| format!("failed to remove {}", final_path.display()))?;
    }
    fs::rename(&temp_path, &final_path).with_context(|| {
        format!(
            "failed to move {} to {}",
            temp_path.display(),
            final_path.display()
        )
    })?;
    Ok(())
}

fn build_published_files(
    versions: &HashMap<String, StoredVersion>,
    packages: &HashMap<String, StoredPackage>,
    catalog_relpath: &str,
    catalog_hash: Vec<u8>,
) -> Vec<PublishedFile> {
    let mut result = Vec::new();
    let mut ordered_versions = versions.values().cloned().collect::<Vec<_>>();
    ordered_versions.sort_by(|left, right| {
        left.published_manifest_relpath
            .cmp(&right.published_manifest_relpath)
    });
    for version in ordered_versions {
        result.push(PublishedFile {
            relpath: version.published_manifest_relpath.clone(),
            kind: "manifest".to_string(),
            owner_package_id: Some(version.package_id.clone()),
            sha256: version.published_manifest_sha256.clone(),
        });
    }

    let mut ordered_packages = packages.values().cloned().collect::<Vec<_>>();
    ordered_packages.retain(|package| !package.version_data_relpath.is_empty());
    ordered_packages
        .sort_by(|left, right| left.version_data_relpath.cmp(&right.version_data_relpath));
    for package in ordered_packages {
        result.push(PublishedFile {
            relpath: package.version_data_relpath.clone(),
            kind: "package".to_string(),
            owner_package_id: Some(package.package_id.clone()),
            sha256: package.package_publish_sha256.clone(),
        });
    }

    result.push(PublishedFile {
        relpath: catalog_relpath.to_string(),
        kind: "catalog".to_string(),
        owner_package_id: None,
        sha256: catalog_hash,
    });

    result
}

fn write_validation_queue(path: PathBuf, requirements: &[ValidationRequirement]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(requirements).context("failed to serialize validation queue")?,
    )
    .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn stored_version_from_computed(snapshot: &ComputedVersionSnapshot) -> StoredVersion {
    StoredVersion {
        version_dir: snapshot.version_dir.clone(),
        package_id: snapshot.package_id.clone(),
        package_version: snapshot.package_version.clone(),
        channel: snapshot.channel.clone(),
        index_projection_json: Some(
            serde_json::to_string(&snapshot.index_projection)
                .expect("failed to serialize version index projection"),
        ),
        installers_json: Some(
            installer_records_to_json(&snapshot.installers)
                .expect("failed to serialize installer records"),
        ),
        published_manifest_relpath: snapshot.published_manifest_relpath.clone(),
        published_manifest_sha256: snapshot.published_manifest_sha256.clone(),
        version_content_sha256: snapshot.version_content_sha256.clone(),
        version_installer_sha256: snapshot.version_installer_sha256.clone(),
        source_file_count: snapshot.source_file_count,
    }
}

fn current_file_to_stored(file: &CurrentFileScan) -> StoredFile {
    StoredFile {
        path: file.path.clone(),
        version_dir: file.version_dir.clone(),
        size: file.size,
        mtime_ns: file.mtime_ns,
        raw_sha256: file.raw_sha256.clone(),
    }
}

fn modified_to_unix_nanos(modified: &SystemTime) -> Result<i64> {
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|_| anyhow!("file modified time predates unix epoch"))?;
    let nanos = duration.as_nanos();
    if nanos > i64::MAX as u128 {
        bail!("file modified time exceeds maximum representable value");
    }
    Ok(nanos as i64)
}

fn unix_now() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| anyhow!("system time predates unix epoch"))?
        .as_secs() as i64)
}

fn sorted_strings<'a>(values: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let mut result = values.into_iter().cloned().collect::<Vec<_>>();
    result.sort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{msix_packaging_available, windows_build_dependencies_available};
    use crate::manifest::{InstallerRecord, VersionIndexProjection, normalize_rel};
    use crate::mszip::decompress_all as decompress_mszip_bytes;
    use rusqlite::{Connection, types::ValueRef};
    use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
    use serde_yaml::Value as YamlValue;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use tempfile::NamedTempFile;
    use walkdir::WalkDir;
    use zip::ZipArchive;

    fn test_build_args(
        repo_dir: PathBuf,
        state_dir: PathBuf,
        index_version: crate::CatalogFormat,
        backend: crate::BackendKind,
    ) -> crate::BuildArgs {
        crate::BuildArgs {
            repo_dir,
            state_dir,
            package_ids: Vec::new(),
            version_dirs: Vec::new(),
            index_version,
            backend,
            force: false,
            dry_run: false,
            no_validation_queue: false,
            display_version_conflict_strategy: crate::DisplayVersionConflictStrategy::Latest,
        }
    }

    fn test_publish_args(
        state_dir: PathBuf,
        out_dir: PathBuf,
        packaging_assets_dir: PathBuf,
    ) -> crate::PublishArgs {
        crate::PublishArgs {
            state_dir,
            out_dir,
            packaging_assets_dir,
            build_id: None,
            force: false,
            dry_run: false,
            sign_pfx_file: None,
            sign_password: None,
            sign_password_env: None,
            timestamp_url: None,
        }
    }

    #[test]
    fn compare_package_versions_prefers_higher_numeric_versions() {
        assert!(crate::version::compare_versions("3.16.2", "3.15.2").is_gt());
        assert!(
            crate::version::compare_versions("11.00.26100.1 (WinBuild.160101.0800)", "2.2.2.1")
                .is_gt()
        );
        assert!(crate::version::compare_versions("3.10", "3.2").is_gt());
    }

    #[test]
    fn validation_queue_tracks_added_installers_per_installer() {
        let version_dir = "manifests/e/Example/App/1.0.0".to_string();
        let old_snapshot =
            synthetic_snapshot(&version_dir, "Example.App", "1.0.0", &["installer-a"], 1);
        let new_snapshot = synthetic_snapshot(
            &version_dir,
            "Example.App",
            "1.0.0",
            &["installer-a", "installer-b"],
            2,
        );

        let version_dirs_to_compare = HashSet::from([version_dir.clone()]);
        let computed_versions = HashMap::from([(version_dir.clone(), new_snapshot.clone())]);
        let previous_versions = HashMap::from([(
            version_dir.clone(),
            stored_version_from_computed(&old_snapshot),
        )]);

        let (version_changes, semantic_changes, validation_requirements) =
            build_version_changes_and_validation_queue(
                &version_dirs_to_compare,
                &computed_versions,
                &previous_versions,
            )
            .unwrap();

        assert_eq!(version_changes.len(), 1);
        assert_eq!(version_changes[0].change_kind, "update");
        assert!(version_changes[0].installer_changed);
        assert!(matches!(
            semantic_changes.first(),
            Some(VersionSemanticChange::Update { .. })
        ));
        assert_eq!(validation_requirements.len(), 1);
        assert_eq!(
            validation_requirements[0].installer.installer_sha256,
            "installer-b"
        );
        assert_eq!(validation_requirements[0].reason, "installer-changed");
    }

    #[test]
    fn arp_display_version_conflicts_keep_highest_package_version() {
        let repo_root = tempfile::tempdir().unwrap();
        let older_version_dir = "manifests/u/Unity/UnityHub/3.15.2".to_string();
        let newer_version_dir = "manifests/u/Unity/UnityHub/3.16.2".to_string();
        let mut computed_versions = HashMap::from([
            (
                older_version_dir.clone(),
                synthetic_snapshot_with_display_version(
                    &older_version_dir,
                    "Unity.UnityHub",
                    "3.15.2",
                    "3.14.4",
                    1,
                ),
            ),
            (
                newer_version_dir.clone(),
                synthetic_snapshot_with_display_version(
                    &newer_version_dir,
                    "Unity.UnityHub",
                    "3.16.2",
                    "3.14.4",
                    2,
                ),
            ),
        ]);
        let current_version_abs = HashMap::from([
            (
                older_version_dir.clone(),
                repo_root.path().join(older_version_dir.as_str()),
            ),
            (
                newer_version_dir.clone(),
                repo_root.path().join(newer_version_dir.as_str()),
            ),
        ]);
        let dirty_version_dirs =
            HashSet::from([older_version_dir.clone(), newer_version_dir.clone()]);
        let previous_versions = HashMap::new();
        let progress = ProgressReporter::new();
        let messages = crate::i18n::Messages::new("en");

        let changed_version_dirs = apply_arp_display_version_policy(
            repo_root.path(),
            &current_version_abs,
            &dirty_version_dirs,
            &previous_versions,
            &mut computed_versions,
            &progress,
            &messages,
            crate::DisplayVersionConflictStrategy::Latest,
        )
        .unwrap();

        assert!(changed_version_dirs.contains(&older_version_dir));
        assert!(changed_version_dirs.contains(&newer_version_dir));
        assert_eq!(
            extract_display_versions_from_manifest_bytes(
                &computed_versions[older_version_dir.as_str()].published_manifest_bytes,
            )
            .unwrap(),
            BTreeSet::new()
        );
        assert_eq!(
            extract_display_versions_from_manifest_bytes(
                &computed_versions[newer_version_dir.as_str()].published_manifest_bytes,
            )
            .unwrap(),
            BTreeSet::from(["3.14.4".to_string()])
        );
    }

    #[test]
    fn arp_display_version_conflicts_can_keep_lowest_package_version() {
        let repo_root = tempfile::tempdir().unwrap();
        let older_version_dir = "manifests/u/Unity/UnityHub/3.15.2".to_string();
        let newer_version_dir = "manifests/u/Unity/UnityHub/3.16.2".to_string();
        let mut computed_versions = HashMap::from([
            (
                older_version_dir.clone(),
                synthetic_snapshot_with_display_version(
                    &older_version_dir,
                    "Unity.UnityHub",
                    "3.15.2",
                    "3.14.4",
                    1,
                ),
            ),
            (
                newer_version_dir.clone(),
                synthetic_snapshot_with_display_version(
                    &newer_version_dir,
                    "Unity.UnityHub",
                    "3.16.2",
                    "3.14.4",
                    2,
                ),
            ),
        ]);
        let current_version_abs = HashMap::from([
            (
                older_version_dir.clone(),
                repo_root.path().join(older_version_dir.as_str()),
            ),
            (
                newer_version_dir.clone(),
                repo_root.path().join(newer_version_dir.as_str()),
            ),
        ]);
        let dirty_version_dirs =
            HashSet::from([older_version_dir.clone(), newer_version_dir.clone()]);
        let previous_versions = HashMap::new();
        let progress = ProgressReporter::new();
        let messages = crate::i18n::Messages::new("en");

        apply_arp_display_version_policy(
            repo_root.path(),
            &current_version_abs,
            &dirty_version_dirs,
            &previous_versions,
            &mut computed_versions,
            &progress,
            &messages,
            crate::DisplayVersionConflictStrategy::Oldest,
        )
        .unwrap();

        assert_eq!(
            extract_display_versions_from_manifest_bytes(
                &computed_versions[older_version_dir.as_str()].published_manifest_bytes,
            )
            .unwrap(),
            BTreeSet::from(["3.14.4".to_string()])
        );
        assert_eq!(
            extract_display_versions_from_manifest_bytes(
                &computed_versions[newer_version_dir.as_str()].published_manifest_bytes,
            )
            .unwrap(),
            BTreeSet::new()
        );
    }

    #[test]
    fn arp_display_version_conflicts_can_strip_all() {
        let repo_root = tempfile::tempdir().unwrap();
        let older_version_dir = "manifests/u/Unity/UnityHub/3.15.2".to_string();
        let newer_version_dir = "manifests/u/Unity/UnityHub/3.16.2".to_string();
        let mut computed_versions = HashMap::from([
            (
                older_version_dir.clone(),
                synthetic_snapshot_with_display_version(
                    &older_version_dir,
                    "Unity.UnityHub",
                    "3.15.2",
                    "3.14.4",
                    1,
                ),
            ),
            (
                newer_version_dir.clone(),
                synthetic_snapshot_with_display_version(
                    &newer_version_dir,
                    "Unity.UnityHub",
                    "3.16.2",
                    "3.14.4",
                    2,
                ),
            ),
        ]);
        let current_version_abs = HashMap::from([
            (
                older_version_dir.clone(),
                repo_root.path().join(older_version_dir.as_str()),
            ),
            (
                newer_version_dir.clone(),
                repo_root.path().join(newer_version_dir.as_str()),
            ),
        ]);
        let dirty_version_dirs =
            HashSet::from([older_version_dir.clone(), newer_version_dir.clone()]);
        let previous_versions = HashMap::new();
        let progress = ProgressReporter::new();
        let messages = crate::i18n::Messages::new("en");

        apply_arp_display_version_policy(
            repo_root.path(),
            &current_version_abs,
            &dirty_version_dirs,
            &previous_versions,
            &mut computed_versions,
            &progress,
            &messages,
            crate::DisplayVersionConflictStrategy::StripAll,
        )
        .unwrap();

        assert_eq!(
            extract_display_versions_from_manifest_bytes(
                &computed_versions[older_version_dir.as_str()].published_manifest_bytes,
            )
            .unwrap(),
            BTreeSet::new()
        );
        assert_eq!(
            extract_display_versions_from_manifest_bytes(
                &computed_versions[newer_version_dir.as_str()].published_manifest_bytes,
            )
            .unwrap(),
            BTreeSet::new()
        );
    }

    #[test]
    fn arp_display_version_conflicts_can_fail() {
        let repo_root = tempfile::tempdir().unwrap();
        let older_version_dir = "manifests/u/Unity/UnityHub/3.15.2".to_string();
        let newer_version_dir = "manifests/u/Unity/UnityHub/3.16.2".to_string();
        let mut computed_versions = HashMap::from([
            (
                older_version_dir.clone(),
                synthetic_snapshot_with_display_version(
                    &older_version_dir,
                    "Unity.UnityHub",
                    "3.15.2",
                    "3.14.4",
                    1,
                ),
            ),
            (
                newer_version_dir.clone(),
                synthetic_snapshot_with_display_version(
                    &newer_version_dir,
                    "Unity.UnityHub",
                    "3.16.2",
                    "3.14.4",
                    2,
                ),
            ),
        ]);
        let current_version_abs = HashMap::from([
            (
                older_version_dir.clone(),
                repo_root.path().join(older_version_dir.as_str()),
            ),
            (
                newer_version_dir.clone(),
                repo_root.path().join(newer_version_dir.as_str()),
            ),
        ]);
        let dirty_version_dirs =
            HashSet::from([older_version_dir.clone(), newer_version_dir.clone()]);
        let previous_versions = HashMap::new();
        let progress = ProgressReporter::new();
        let messages = crate::i18n::Messages::new("en");

        let error = apply_arp_display_version_policy(
            repo_root.path(),
            &current_version_abs,
            &dirty_version_dirs,
            &previous_versions,
            &mut computed_versions,
            &progress,
            &messages,
            crate::DisplayVersionConflictStrategy::Error,
        )
        .expect_err("error strategy should fail on conflicting DisplayVersion");

        let rendered = format!("{error:#}");
        assert!(rendered.contains("Unity.UnityHub"));
        assert!(rendered.contains("3.14.4"));
        assert!(rendered.contains("3.15.2"));
        assert!(rendered.contains("3.16.2"));
    }

    #[test]
    fn builds_fixture_repo_end_to_end_on_windows() {
        if !cfg!(windows) {
            return;
        }

        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("data")
            .join("e2e-repo");
        let workspace_root = crate::adapter::resolve_workspace_root(Some(&repo)).unwrap();
        if !windows_build_dependencies_available(&workspace_root) {
            return;
        }

        let state = workspace_root.join("target").join("itest-state");
        let out = workspace_root.join("target").join("itest-out");

        if state.exists() {
            let _ = fs::remove_dir_all(&state);
        }
        if out.exists() {
            let _ = fs::remove_dir_all(&out);
        }

        let args = test_build_args(
            repo.clone(),
            state.clone(),
            crate::CatalogFormat::V2,
            crate::BackendKind::Wingetutil,
        );

        run_build(args, crate::i18n::Messages::new("en")).unwrap();
        run_publish(
            test_publish_args(state.clone(), out.clone(), repo.join("packaging")),
            crate::i18n::Messages::new("en"),
        )
        .unwrap();

        assert!(out.join("source2.msix").is_file());
        assert!(out.join("packages").is_dir());
        assert!(out.join("manifests").is_dir());
        assert!(state.join("writer").join("mutable-v2.db").is_file());
    }

    #[test]
    fn builds_fixture_repo_with_rust_backend_on_windows() {
        if !cfg!(windows) {
            return;
        }

        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("data")
            .join("e2e-repo");
        let workspace_root = crate::adapter::resolve_workspace_root(Some(&repo)).unwrap();
        if !windows_build_dependencies_available(&workspace_root) {
            return;
        }

        let state = workspace_root
            .join("target")
            .join("itest-rust-backend-state");
        let out = workspace_root.join("target").join("itest-rust-backend-out");

        if state.exists() {
            let _ = fs::remove_dir_all(&state);
        }
        if out.exists() {
            let _ = fs::remove_dir_all(&out);
        }

        let args = test_build_args(
            repo.clone(),
            state.clone(),
            crate::CatalogFormat::V2,
            crate::BackendKind::Rust,
        );

        run_build(args, crate::i18n::Messages::new("en")).unwrap();
        run_publish(
            test_publish_args(state.clone(), out.clone(), repo.join("packaging")),
            crate::i18n::Messages::new("en"),
        )
        .unwrap();

        assert!(out.join("source2.msix").is_file());
        assert!(out.join("packages").is_dir());
        assert!(out.join("manifests").is_dir());
        assert!(state.join("state.sqlite").is_file());
    }

    #[test]
    fn builds_fixture_repo_with_rust_v1_backend_when_packager_is_available() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("data")
            .join("e2e-repo");
        let workspace_root = crate::adapter::resolve_workspace_root(Some(&repo)).unwrap();
        if !msix_packaging_available(&workspace_root) {
            return;
        }

        let state = workspace_root.join("target").join("itest-rust-v1-state");
        let out = workspace_root.join("target").join("itest-rust-v1-out");

        if state.exists() {
            let _ = fs::remove_dir_all(&state);
        }
        if out.exists() {
            let _ = fs::remove_dir_all(&out);
        }

        let args = test_build_args(
            repo.clone(),
            state.clone(),
            crate::CatalogFormat::V1,
            crate::BackendKind::Rust,
        );

        run_build(args, crate::i18n::Messages::new("en")).unwrap();
        run_publish(
            test_publish_args(state.clone(), out.clone(), repo.join("packaging")),
            crate::i18n::Messages::new("en"),
        )
        .unwrap();

        assert!(out.join("source.msix").is_file());
        assert!(!out.join("packages").exists());
        assert!(out.join("manifests").is_dir());
        assert!(state.join("state.sqlite").is_file());
    }

    #[test]
    fn rust_backend_backfills_missing_index_projections_on_windows() {
        if !cfg!(windows) {
            return;
        }

        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("data")
            .join("e2e-repo");
        let workspace_root = crate::adapter::resolve_workspace_root(Some(&repo)).unwrap();
        if !windows_build_dependencies_available(&workspace_root) {
            return;
        }

        let state = workspace_root
            .join("target")
            .join("itest-rust-backfill-state");
        let out = workspace_root
            .join("target")
            .join("itest-rust-backfill-out");

        if state.exists() {
            let _ = fs::remove_dir_all(&state);
        }
        if out.exists() {
            let _ = fs::remove_dir_all(&out);
        }

        let initial_args = test_build_args(
            repo.clone(),
            state.clone(),
            crate::CatalogFormat::V2,
            crate::BackendKind::Rust,
        );
        run_build(initial_args, crate::i18n::Messages::new("en")).unwrap();

        let conn = Connection::open(state.join("state.sqlite")).unwrap();
        conn.execute(
            "UPDATE versions_current SET index_projection_json = NULL",
            [],
        )
        .unwrap();
        drop(conn);

        let backfill_args = test_build_args(
            repo.clone(),
            state.clone(),
            crate::CatalogFormat::V2,
            crate::BackendKind::Rust,
        );
        run_build(backfill_args, crate::i18n::Messages::new("en")).unwrap();

        let store = StateStore::open(&state).unwrap();
        let versions = store.load_versions_current().unwrap();
        assert!(!versions.is_empty());
        assert!(
            versions
                .values()
                .all(|version| version.index_projection_json.is_some())
        );
        assert!(state.join("staging").join("build-2").is_dir());
    }

    #[test]
    fn rust_backend_matches_wingetutil_on_fixture_windows() {
        if !cfg!(windows) {
            return;
        }

        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("data")
            .join("e2e-repo");
        let workspace_root = crate::adapter::resolve_workspace_root(Some(&repo)).unwrap();
        if !windows_build_dependencies_available(&workspace_root) {
            return;
        }

        let wingetutil_state = workspace_root
            .join("target")
            .join("itest-parity-wingetutil-state");
        let wingetutil_out = workspace_root
            .join("target")
            .join("itest-parity-wingetutil-out");
        let rust_state = workspace_root
            .join("target")
            .join("itest-parity-rust-state");
        let rust_out = workspace_root.join("target").join("itest-parity-rust-out");

        for path in [&wingetutil_state, &wingetutil_out, &rust_state, &rust_out] {
            if path.exists() {
                let _ = fs::remove_dir_all(path);
            }
        }

        run_build(
            test_build_args(
                repo.clone(),
                wingetutil_state.clone(),
                crate::CatalogFormat::V2,
                crate::BackendKind::Wingetutil,
            ),
            crate::i18n::Messages::new("en"),
        )
        .unwrap();
        run_publish(
            test_publish_args(
                wingetutil_state,
                wingetutil_out.clone(),
                repo.join("packaging"),
            ),
            crate::i18n::Messages::new("en"),
        )
        .unwrap();

        run_build(
            test_build_args(
                repo.clone(),
                rust_state.clone(),
                crate::CatalogFormat::V2,
                crate::BackendKind::Rust,
            ),
            crate::i18n::Messages::new("en"),
        )
        .unwrap();
        run_publish(
            test_publish_args(rust_state, rust_out.clone(), repo.join("packaging")),
            crate::i18n::Messages::new("en"),
        )
        .unwrap();

        assert_eq!(
            collect_output_files(&wingetutil_out, "manifests").unwrap(),
            collect_output_files(&rust_out, "manifests").unwrap()
        );
        assert_eq!(
            collect_semantic_package_files(&wingetutil_out).unwrap(),
            collect_semantic_package_files(&rust_out).unwrap()
        );
        compare_msix_indices(
            &wingetutil_out.join("source2.msix"),
            &rust_out.join("source2.msix"),
        )
        .unwrap();
    }

    fn collect_output_files(out_root: &Path, subdir: &str) -> Result<BTreeMap<String, Vec<u8>>> {
        let root = out_root.join(subdir);
        let mut files = BTreeMap::new();
        if !root.is_dir() {
            return Ok(files);
        }

        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            let rel = normalize_rel(&path.strip_prefix(out_root)?.to_string_lossy());
            files.insert(
                rel,
                fs::read(path).with_context(|| format!("failed to read {}", path.display()))?,
            );
        }

        Ok(files)
    }

    fn synthetic_snapshot(
        version_dir: &str,
        package_id: &str,
        package_version: &str,
        installer_hashes: &[&str],
        content_marker: u8,
    ) -> ComputedVersionSnapshot {
        let installers = installer_hashes
            .iter()
            .map(|installer_sha256| InstallerRecord {
                installer_sha256: (*installer_sha256).to_string(),
                installer_url: Some(format!("https://example.invalid/{installer_sha256}.exe")),
                architecture: Some("x64".to_string()),
                installer_type: Some("exe".to_string()),
                installer_locale: Some("en-US".to_string()),
                scope: Some("user".to_string()),
                package_family_name: None,
                product_codes: Vec::new(),
            })
            .collect::<Vec<_>>();

        ComputedVersionSnapshot {
            version_dir: version_dir.to_string(),
            package_id: package_id.to_string(),
            package_version: package_version.to_string(),
            channel: String::new(),
            index_projection: VersionIndexProjection {
                package_name: package_id.to_string(),
                ..VersionIndexProjection::default()
            },
            version_content_sha256: vec![content_marker; 32],
            installers: installers.clone(),
            version_installer_sha256: sha256_bytes(
                serde_json::to_string(&installers).unwrap().as_bytes(),
            ),
            published_manifest_sha256: vec![content_marker + 1; 32],
            published_manifest_relpath: format!(
                "manifests/e/Example/App/{package_version}/{content_marker:02x}.yaml"
            ),
            published_manifest_bytes: format!(
                "PackageIdentifier: {package_id}\nPackageVersion: {package_version}\n"
            )
            .into_bytes(),
            source_file_count: 1,
        }
    }

    fn synthetic_snapshot_with_display_version(
        version_dir: &str,
        package_id: &str,
        package_version: &str,
        display_version: &str,
        content_marker: u8,
    ) -> ComputedVersionSnapshot {
        let snapshot = synthetic_snapshot(
            version_dir,
            package_id,
            package_version,
            &[&format!("installer-{content_marker:02x}")],
            content_marker,
        );
        let manifest_bytes = format!(
            concat!(
                "PackageIdentifier: {package_id}\n",
                "PackageVersion: \"{package_version}\"\n",
                "ManifestVersion: 1.10.0\n",
                "ManifestType: merged\n",
                "AppsAndFeaturesEntries:\n",
                "  - DisplayVersion: \"{display_version}\"\n",
                "Installers:\n",
                "  - Architecture: x64\n",
                "    InstallerType: exe\n",
                "    InstallerUrl: https://example.invalid/{content_marker:02x}.exe\n",
                "    InstallerSha256: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n"
            ),
            package_id = package_id,
            package_version = package_version,
            display_version = display_version,
            content_marker = content_marker,
        )
        .into_bytes();
        let published_manifest_sha256 = sha256_bytes(&manifest_bytes);

        ComputedVersionSnapshot {
            published_manifest_sha256,
            published_manifest_bytes: manifest_bytes,
            ..snapshot
        }
    }

    fn collect_semantic_package_files(out_root: &Path) -> Result<BTreeMap<String, JsonValue>> {
        let root = out_root.join("packages");
        let mut files = BTreeMap::new();
        if !root.is_dir() {
            return Ok(files);
        }

        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            if path.file_name().and_then(|name| name.to_str()) != Some("versionData.mszyml") {
                continue;
            }

            let rel = normalize_rel(&path.strip_prefix(out_root)?.to_string_lossy());
            let bytes =
                fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
            let decompressed = decompress_mszip_bytes(&bytes)?;
            let yaml: YamlValue = serde_yaml::from_slice(&decompressed)
                .context("failed to parse versionData YAML")?;
            files.insert(rel, canonicalize_yaml_value(&yaml));
        }

        Ok(files)
    }

    fn compare_msix_indices(left_msix: &Path, right_msix: &Path) -> Result<()> {
        let left_db = extract_msix_entry_to_temp(left_msix, "Public/index.db")?;
        let right_db = extract_msix_entry_to_temp(right_msix, "Public/index.db")?;
        let left = Connection::open(left_db.path())
            .with_context(|| format!("failed to open {}", left_db.path().display()))?;
        let right = Connection::open(right_db.path())
            .with_context(|| format!("failed to open {}", right_db.path().display()))?;

        let expected_tables = [
            "commands2",
            "commands2_map",
            "metadata",
            "norm_names2",
            "norm_publishers2",
            "packages",
            "pfns2",
            "productcodes2",
            "tags2",
            "tags2_map",
            "upgradecodes2",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();

        assert_eq!(load_table_names(&left)?, expected_tables);
        assert_eq!(load_table_names(&right)?, expected_tables);

        compare_query_rows(
            &left,
            &right,
            "metadata",
            "SELECT name, value FROM metadata WHERE name NOT IN ('databaseIdentifier', 'lastwritetime') ORDER BY name",
        )?;
        compare_query_rows(
            &left,
            &right,
            "packages",
            "SELECT id, name, COALESCE(moniker, ''), latest_version, COALESCE(arp_min_version, ''), COALESCE(arp_max_version, ''), hex(hash) FROM packages ORDER BY id",
        )?;
        compare_query_rows(
            &left,
            &right,
            "tags",
            "SELECT p.id, t.tag FROM tags2_map m JOIN tags2 t ON m.tag = t.rowid JOIN packages p ON m.package = p.rowid ORDER BY p.id, t.tag",
        )?;
        compare_query_rows(
            &left,
            &right,
            "commands",
            "SELECT p.id, c.command FROM commands2_map m JOIN commands2 c ON m.command = c.rowid JOIN packages p ON m.package = p.rowid ORDER BY p.id, c.command",
        )?;
        compare_query_rows(
            &left,
            &right,
            "pfns",
            "SELECT p.id, f.pfn FROM pfns2 f JOIN packages p ON f.package = p.rowid ORDER BY p.id, f.pfn",
        )?;
        compare_query_rows(
            &left,
            &right,
            "productcodes",
            "SELECT p.id, f.productcode FROM productcodes2 f JOIN packages p ON f.package = p.rowid ORDER BY p.id, f.productcode",
        )?;
        compare_query_rows(
            &left,
            &right,
            "upgradecodes",
            "SELECT p.id, f.upgradecode FROM upgradecodes2 f JOIN packages p ON f.package = p.rowid ORDER BY p.id, f.upgradecode",
        )?;
        compare_query_rows(
            &left,
            &right,
            "norm_names",
            "SELECT p.id, f.norm_name FROM norm_names2 f JOIN packages p ON f.package = p.rowid ORDER BY p.id, f.norm_name",
        )?;
        compare_query_rows(
            &left,
            &right,
            "norm_publishers",
            "SELECT p.id, f.norm_publisher FROM norm_publishers2 f JOIN packages p ON f.package = p.rowid ORDER BY p.id, f.norm_publisher",
        )?;

        Ok(())
    }

    fn extract_msix_entry_to_temp(msix_path: &Path, entry_name: &str) -> Result<NamedTempFile> {
        let file = File::open(msix_path)
            .with_context(|| format!("failed to open {}", msix_path.display()))?;
        let mut archive = ZipArchive::new(file)
            .with_context(|| format!("failed to read {}", msix_path.display()))?;
        let mut entry = archive
            .by_name(entry_name)
            .with_context(|| format!("failed to locate {entry_name} in {}", msix_path.display()))?;
        let mut temp = NamedTempFile::new().context("failed to create temp file")?;
        std::io::copy(&mut entry, &mut temp).with_context(|| {
            format!(
                "failed to extract {entry_name} from {}",
                msix_path.display()
            )
        })?;
        temp.flush().context("failed to flush temp file")?;
        Ok(temp)
    }

    fn load_table_names(conn: &Connection) -> Result<BTreeSet<String>> {
        let mut statement = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut tables = BTreeSet::new();
        for row in rows {
            tables.insert(row?);
        }
        Ok(tables)
    }

    fn compare_query_rows(
        left: &Connection,
        right: &Connection,
        label: &str,
        sql: &str,
    ) -> Result<()> {
        let left_rows = query_rows(left, sql)?;
        let right_rows = query_rows(right, sql)?;
        assert_eq!(left_rows, right_rows, "mismatch in {label}");
        Ok(())
    }

    fn query_rows(conn: &Connection, sql: &str) -> Result<Vec<Vec<String>>> {
        let mut statement = conn.prepare(sql)?;
        let column_count = statement.column_count();
        let mut rows = statement.query([])?;
        let mut result = Vec::new();

        while let Some(row) = rows.next()? {
            let mut normalized = Vec::with_capacity(column_count);
            for index in 0..column_count {
                let value = row.get_ref(index)?;
                normalized.push(normalize_sql_value(value));
            }
            result.push(normalized);
        }

        Ok(result)
    }

    fn normalize_sql_value(value: ValueRef<'_>) -> String {
        match value {
            ValueRef::Null => "null".to_string(),
            ValueRef::Integer(value) => format!("i:{value}"),
            ValueRef::Real(value) => format!("f:{value}"),
            ValueRef::Text(value) => format!("t:{}", String::from_utf8_lossy(value)),
            ValueRef::Blob(value) => format!("b:{}", hex::encode_upper(value)),
        }
    }

    fn canonicalize_yaml_value(value: &YamlValue) -> JsonValue {
        match value {
            YamlValue::Null => JsonValue::Null,
            YamlValue::Bool(value) => JsonValue::Bool(*value),
            YamlValue::Number(value) => {
                if let Some(integer) = value.as_i64() {
                    JsonValue::Number(JsonNumber::from(integer))
                } else if let Some(integer) = value.as_u64() {
                    JsonValue::Number(JsonNumber::from(integer))
                } else if let Some(float) = value.as_f64() {
                    JsonValue::Number(
                        JsonNumber::from_f64(float)
                            .expect("YAML number should be representable as JSON"),
                    )
                } else {
                    JsonValue::String(value.to_string())
                }
            }
            YamlValue::String(value) => JsonValue::String(value.clone()),
            YamlValue::Sequence(items) => JsonValue::Array(
                items
                    .iter()
                    .map(canonicalize_yaml_value)
                    .collect::<Vec<_>>(),
            ),
            YamlValue::Mapping(entries) => {
                let mut normalized = entries
                    .iter()
                    .map(|(key, value)| (yaml_key_to_string(key), canonicalize_yaml_value(value)))
                    .collect::<Vec<_>>();
                normalized.sort_by(|left, right| left.0.cmp(&right.0));

                let mut map = JsonMap::new();
                for (key, value) in normalized {
                    map.insert(key, value);
                }
                JsonValue::Object(map)
            }
            YamlValue::Tagged(tagged) => canonicalize_yaml_value(&tagged.value),
        }
    }

    fn yaml_key_to_string(value: &YamlValue) -> String {
        match value {
            YamlValue::Null => "null".to_string(),
            YamlValue::Bool(value) => value.to_string(),
            YamlValue::Number(value) => value.to_string(),
            YamlValue::String(value) => value.clone(),
            YamlValue::Sequence(_) | YamlValue::Mapping(_) | YamlValue::Tagged(_) => {
                serde_yaml::to_string(value)
                    .expect("failed to serialize YAML key")
                    .trim()
                    .to_string()
            }
        }
    }
}
