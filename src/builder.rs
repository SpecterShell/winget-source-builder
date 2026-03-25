use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail, ensure};
use log::info;
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::BuildArgs;
use crate::adapter::{
    AdapterOperation, AdapterRequest, absolute_string, resolve_workspace_root, run_adapter,
};
use crate::i18n::Messages;
use crate::manifest::{
    ComputedVersionSnapshot, ManifestWarning, ValidationRequirement,
    compute_version_snapshot_with_warnings, extract_display_versions_from_manifest_bytes,
    normalize_rel, retain_display_versions_in_snapshot, scan_root, sha256_bytes,
};
use crate::progress::ProgressReporter;
use crate::state::{
    BuildPackageChange, BuildVersionChange, CurrentStateUpdate, PublishedFile, StateStore,
    StoredFile, StoredPackage, StoredVersion,
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
    Add(ComputedVersionSnapshot),
    Update {
        old: StoredVersion,
        new: ComputedVersionSnapshot,
    },
    Remove(StoredVersion),
    Noop,
}

pub fn run_build(args: BuildArgs, messages: Messages) -> Result<()> {
    info!("{}", messages.build_started(&args.repo, &args.out));
    let mut state = StateStore::open(&args.state)?;
    let started_unix = unix_now()?;
    let build_id = state.begin_build(started_unix)?;

    let result = run_build_inner(&args, &mut state, build_id, messages);
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

fn run_build_inner(
    args: &BuildArgs,
    state: &mut StateStore,
    build_id: i64,
    messages: Messages,
) -> Result<()> {
    match args.format {
        crate::CatalogFormat::V2 => {}
    }

    let progress = ProgressReporter::new();

    let repo_root = args
        .repo
        .canonicalize()
        .with_context(|| format!("failed to resolve repo path {}", args.repo.display()))?;
    let out_root = args.out.clone();
    let scan_root = scan_root(&repo_root);
    let workspace_root = resolve_workspace_root(Some(&repo_root))?;

    info!("{}", messages.scanning_repository(&scan_root));

    let previous_files = state.load_files_current()?;
    let previous_versions = state.load_versions_current()?;
    let previous_packages = state.load_packages_current()?;
    let previous_published_files = state.load_published_files_current()?;
    let last_successful_unix = state.last_successful_build_epoch()?;

    let mut current_files = scan_yaml_files(&repo_root, &scan_root, &progress, &messages)?;
    fill_file_hashes(&mut current_files, &previous_files, &progress, &messages)?;

    let dirty_version_dirs = determine_dirty_version_dirs(&current_files, &previous_files);
    let current_version_dirs = current_files
        .values()
        .map(|file| file.version_dir.clone())
        .collect::<HashSet<_>>();

    let current_version_abs = current_files
        .values()
        .map(|file| (file.version_dir.clone(), file.version_dir_abs.clone()))
        .collect::<HashMap<_, _>>();

    let dirty_existing_version_dirs = dirty_version_dirs
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
    let arp_policy_changed_version_dirs = apply_arp_display_version_policy(
        &repo_root,
        &current_version_abs,
        &dirty_version_dirs,
        &previous_versions,
        &mut computed_versions,
        &progress,
        &messages,
    )?;
    let version_dirs_to_compare = dirty_version_dirs
        .union(&arp_policy_changed_version_dirs)
        .cloned()
        .collect::<HashSet<_>>();

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
                    installer_changed: true,
                    old_content_sha256: None,
                    new_content_sha256: Some(new_version.version_content_sha256.clone()),
                });
                semantic_changes.push(VersionSemanticChange::Add(new_version.clone()));
                validation_requirements.push(ValidationRequirement {
                    package_id: new_version.package_id.clone(),
                    package_version: new_version.package_version.clone(),
                    channel: new_version.channel.clone(),
                    version_installer_sha256: hex::encode(&new_version.version_installer_sha256),
                    reason: "added".to_string(),
                });
            }
            (Some(old_version), None) => {
                version_changes.push(BuildVersionChange {
                    version_dir: version_dir.clone(),
                    package_id: old_version.package_id.clone(),
                    change_kind: "remove".to_string(),
                    content_changed: true,
                    installer_changed: true,
                    old_content_sha256: Some(old_version.version_content_sha256.clone()),
                    new_content_sha256: None,
                });
                semantic_changes.push(VersionSemanticChange::Remove(old_version.clone()));
                validation_requirements.push(ValidationRequirement {
                    package_id: old_version.package_id.clone(),
                    package_version: old_version.package_version.clone(),
                    channel: old_version.channel.clone(),
                    version_installer_sha256: hex::encode(&old_version.version_installer_sha256),
                    reason: "removed".to_string(),
                });
            }
            (Some(old_version), Some(new_version)) => {
                let content_changed = stored_version_differs(old_version, new_version);
                let installer_changed =
                    old_version.version_installer_sha256 != new_version.version_installer_sha256;

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
                        old: old_version.clone(),
                        new: new_version.clone(),
                    });
                    if installer_changed {
                        validation_requirements.push(ValidationRequirement {
                            package_id: new_version.package_id.clone(),
                            package_version: new_version.package_version.clone(),
                            channel: new_version.channel.clone(),
                            version_installer_sha256: hex::encode(
                                &new_version.version_installer_sha256,
                            ),
                            reason: "installer-changed".to_string(),
                        });
                    }
                } else {
                    version_changes.push(BuildVersionChange {
                        version_dir: version_dir.clone(),
                        package_id: old_version.package_id.clone(),
                        change_kind: "noop".to_string(),
                        content_changed: false,
                        installer_changed: false,
                        old_content_sha256: Some(old_version.version_content_sha256.clone()),
                        new_content_sha256: Some(new_version.version_content_sha256.clone()),
                    });
                    semantic_changes.push(VersionSemanticChange::Noop);
                }
            }
            (None, None) => {}
        }
    }

    state.record_version_changes(build_id, &version_changes)?;
    let validation_queue_path = state.validation_queue_path();
    write_validation_queue(validation_queue_path.clone(), &validation_requirements)?;
    info!(
        "{}",
        messages.validation_queue_written(validation_requirements.len(), &validation_queue_path)
    );

    let semantic_version_changes = semantic_changes
        .iter()
        .filter(|change| !matches!(change, VersionSemanticChange::Noop))
        .count();

    if semantic_version_changes == 0 {
        info!("{}", messages.no_semantic_changes());
        let source2_present = out_root.join("source2.msix").is_file();
        if !source2_present && !previous_versions.is_empty() {
            bail!(
                "no semantic changes detected, but {} is missing; a full rebuild is required",
                out_root.join("source2.msix").display()
            );
        }

        let finished_unix = unix_now()?;
        let final_files = current_files
            .values()
            .map(current_file_to_stored)
            .collect::<Vec<_>>();
        let final_versions = previous_versions.values().cloned().collect::<Vec<_>>();
        let final_packages = previous_packages.values().cloned().collect::<Vec<_>>();
        let final_published_files = previous_published_files
            .values()
            .cloned()
            .collect::<Vec<_>>();
        state.replace_current_state(
            build_id,
            CurrentStateUpdate {
                finished_unix,
                last_successful_unix: finished_unix,
                files: &final_files,
                versions: &final_versions,
                packages: &final_packages,
                published_files: &final_published_files,
            },
        )?;
        info!("{}", messages.build_completed(&out_root, &args.state));
        return Ok(());
    }

    info!(
        "{}",
        messages.staging_publish_tree(semantic_version_changes)
    );
    let stage_root = state.staging_root().join(format!("build-{build_id}"));
    if stage_root.exists() {
        fs::remove_dir_all(&stage_root)
            .with_context(|| format!("failed to clear {}", stage_root.display()))?;
    }
    fs::create_dir_all(&stage_root)
        .with_context(|| format!("failed to create {}", stage_root.display()))?;

    let candidate_db_path = stage_root.join("mutable-v2.db");
    let publish_db_path = stage_root.join("index-publish.db");

    let mut adapter_remove_ops = Vec::<AdapterOperation>::new();
    let mut adapter_add_ops = Vec::<AdapterOperation>::new();
    let mut changed_manifest_relpaths = BTreeSet::<String>::new();
    let mut deleted_manifest_relpaths = BTreeSet::<String>::new();
    let mut touched_packages = BTreeSet::<String>::new();

    let staging_progress = progress.bar(
        semantic_version_changes,
        messages.progress_staging_manifests(),
    );
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
                changed_manifest_relpaths.insert(new_version.published_manifest_relpath.clone());
                touched_packages.insert(new_version.package_id.clone());
                ProgressReporter::inc(&staging_progress, 1);
            }
            VersionSemanticChange::Update { old, new } => {
                let old_abs = out_root.join(&old.published_manifest_relpath);
                ensure!(
                    old_abs.is_file(),
                    "existing published manifest is missing: {}",
                    old_abs.display()
                );
                stage_manifest(&stage_root, new)?;
                adapter_remove_ops.push(AdapterOperation {
                    kind: "remove".to_string(),
                    manifest_path: absolute_string(&old_abs),
                    relative_path: old.published_manifest_relpath.clone(),
                });
                adapter_add_ops.push(AdapterOperation {
                    kind: "add".to_string(),
                    manifest_path: absolute_string(
                        &stage_root.join(&new.published_manifest_relpath),
                    ),
                    relative_path: new.published_manifest_relpath.clone(),
                });
                changed_manifest_relpaths.insert(new.published_manifest_relpath.clone());
                if old.published_manifest_relpath != new.published_manifest_relpath {
                    deleted_manifest_relpaths.insert(old.published_manifest_relpath.clone());
                }
                touched_packages.insert(old.package_id.clone());
                touched_packages.insert(new.package_id.clone());
                ProgressReporter::inc(&staging_progress, 1);
            }
            VersionSemanticChange::Remove(old) => {
                let old_abs = out_root.join(&old.published_manifest_relpath);
                ensure!(
                    old_abs.is_file(),
                    "existing published manifest is missing: {}",
                    old_abs.display()
                );
                adapter_remove_ops.push(AdapterOperation {
                    kind: "remove".to_string(),
                    manifest_path: absolute_string(&old_abs),
                    relative_path: old.published_manifest_relpath.clone(),
                });
                deleted_manifest_relpaths.insert(old.published_manifest_relpath.clone());
                touched_packages.insert(old.package_id.clone());
                ProgressReporter::inc(&staging_progress, 1);
            }
            VersionSemanticChange::Noop => {}
        }
    }
    ProgressReporter::finish(staging_progress);

    let mut adapter_ops = adapter_remove_ops;
    adapter_ops.extend(adapter_add_ops);

    let adapter_request = AdapterRequest {
        mutable_db_path: absolute_string(&state.mutable_db_path()),
        candidate_db_path: absolute_string(&candidate_db_path),
        publish_db_path: absolute_string(&publish_db_path),
        stage_root: absolute_string(&stage_root),
        package_update_tracking_base_time: last_successful_unix,
        operations: adapter_ops,
    };

    info!("{}", messages.running_adapter());
    let adapter_progress = progress.spinner(messages.progress_running_adapter());
    run_adapter(&workspace_root, &adapter_request, &stage_root)?;
    ProgressReporter::finish(adapter_progress);

    let staged_package_files = scan_staged_package_files(&stage_root)?;
    let staged_source2 = stage_root.join("source2.msix");
    ensure!(
        staged_source2.is_file(),
        "WinGetUtil packaging did not produce source2.msix"
    );

    let mut final_versions = previous_versions.clone();
    for change in &semantic_changes {
        match change {
            VersionSemanticChange::Add(new) => {
                final_versions.insert(new.version_dir.clone(), stored_version_from_computed(new));
            }
            VersionSemanticChange::Update { new, .. } => {
                final_versions.insert(new.version_dir.clone(), stored_version_from_computed(new));
            }
            VersionSemanticChange::Remove(old) => {
                final_versions.remove(&old.version_dir);
            }
            VersionSemanticChange::Noop => {}
        }
    }

    let mut package_changes = Vec::<BuildPackageChange>::new();
    let final_packages_map = build_final_packages(
        &final_versions,
        &previous_packages,
        &staged_package_files,
        &touched_packages,
        &validation_requirements,
        &mut package_changes,
    )?;
    state.record_package_changes(build_id, &package_changes)?;

    info!("{}", messages.committing_output(&out_root));
    let commit_progress = progress.spinner(messages.progress_committing_output());
    commit_output_tree(
        &stage_root,
        &out_root,
        &changed_manifest_relpaths,
        &deleted_manifest_relpaths,
        &staged_package_files,
        &previous_packages,
        &final_packages_map,
    )?;
    ProgressReporter::finish(commit_progress);

    commit_mutable_db(state.mutable_db_path(), &candidate_db_path)?;

    let source2_hash = sha256_bytes(
        &fs::read(&staged_source2)
            .with_context(|| format!("failed to read {}", staged_source2.display()))?,
    );

    let final_published_files =
        build_published_files(&final_versions, &final_packages_map, source2_hash);
    let final_files = current_files
        .values()
        .map(current_file_to_stored)
        .collect::<Vec<_>>();
    let final_versions_vec = final_versions.values().cloned().collect::<Vec<_>>();
    let final_packages_vec = final_packages_map.values().cloned().collect::<Vec<_>>();

    let finished_unix = unix_now()?;
    state.replace_current_state(
        build_id,
        CurrentStateUpdate {
            finished_unix,
            last_successful_unix: finished_unix,
            files: &final_files,
            versions: &final_versions_vec,
            packages: &final_packages_vec,
            published_files: &final_published_files,
        },
    )?;

    if stage_root.exists() {
        let _ = fs::remove_dir_all(&stage_root);
    }

    info!("{}", messages.build_completed(&out_root, &args.state));

    Ok(())
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
    let to_hash = current_files
        .iter()
        .filter_map(|(path, file)| {
            previous_files
                .get(path)
                .and_then(|previous| {
                    if previous.size == file.size && previous.mtime_ns == file.mtime_ns {
                        None
                    } else {
                        Some((path.clone(), file.abs_path.clone()))
                    }
                })
                .or_else(|| Some((path.clone(), file.abs_path.clone())))
        })
        .collect::<Vec<_>>();

    let hash_progress = progress.bar(to_hash.len(), messages.progress_hashing_files());
    let hashed = to_hash
        .par_iter()
        .map(|(path, abs_path)| {
            let bytes = fs::read(abs_path)
                .with_context(|| format!("failed to read manifest {}", abs_path.display()))?;
            ProgressReporter::inc(&hash_progress, 1);
            Ok::<_, anyhow::Error>((path.clone(), sha256_bytes(&bytes)))
        })
        .collect::<Vec<_>>();

    let mut hash_map = HashMap::<String, Vec<u8>>::new();
    for item in hashed {
        let (path, hash) = item?;
        hash_map.insert(path, hash);
    }

    for (path, file) in current_files.iter_mut() {
        if let Some(previous) = previous_files.get(path) {
            if previous.size == file.size && previous.mtime_ns == file.mtime_ns {
                file.raw_sha256 = previous.raw_sha256.clone();
            } else {
                file.raw_sha256 = hash_map
                    .remove(path)
                    .ok_or_else(|| anyhow!("missing hash for {}", file.abs_path.display()))?;
            }
        } else {
            file.raw_sha256 = hash_map
                .remove(path)
                .ok_or_else(|| anyhow!("missing hash for {}", file.abs_path.display()))?;
        }
    }

    ProgressReporter::finish(hash_progress);

    Ok(())
}

fn determine_dirty_version_dirs(
    current_files: &HashMap<String, CurrentFileScan>,
    previous_files: &HashMap<String, StoredFile>,
) -> HashSet<String> {
    let mut dirty = HashSet::new();

    for (path, current) in current_files {
        match previous_files.get(path) {
            Some(previous) if previous.raw_sha256 == current.raw_sha256 => {}
            _ => {
                dirty.insert(current.version_dir.clone());
            }
        }
    }

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

fn apply_arp_display_version_policy(
    repo_root: &Path,
    current_version_abs: &HashMap<String, PathBuf>,
    dirty_version_dirs: &HashSet<String>,
    previous_versions: &HashMap<String, StoredVersion>,
    computed_versions: &mut HashMap<String, ComputedVersionSnapshot>,
    progress: &ProgressReporter,
    messages: &Messages,
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

            let winner = contenders
                .iter()
                .max_by(|left, right| compare_snapshot_versions(computed_versions, left, right))
                .cloned()
                .ok_or_else(|| anyhow!("display version contenders unexpectedly empty"))?;
            let stripped_versions = contenders
                .iter()
                .filter(|version_dir| *version_dir != &winner)
                .map(|version_dir| describe_snapshot_version(computed_versions, version_dir))
                .collect::<Vec<_>>();

            if !stripped_versions.is_empty() {
                let retained_version = describe_snapshot_version(computed_versions, &winner);
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

fn compare_snapshot_versions(
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

    compare_package_versions(&left.package_version, &right.package_version)
        .then_with(|| left.channel.cmp(&right.channel))
        .then_with(|| left_version_dir.cmp(right_version_dir))
}

fn compare_package_versions(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let left_tokens = tokenize_version(left);
    let right_tokens = tokenize_version(right);

    for (left_token, right_token) in left_tokens.iter().zip(right_tokens.iter()) {
        let ordering = match (left_token, right_token) {
            (VersionToken::Number(left), VersionToken::Number(right)) => {
                compare_numeric_tokens(left, right)
            }
            (VersionToken::Text(left), VersionToken::Text(right)) => {
                left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
            }
            (VersionToken::Number(_), VersionToken::Text(_)) => Ordering::Greater,
            (VersionToken::Text(_), VersionToken::Number(_)) => Ordering::Less,
        };

        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    left_tokens.len().cmp(&right_tokens.len())
}

fn compare_numeric_tokens(left: &str, right: &str) -> std::cmp::Ordering {
    let left = left.trim_start_matches('0');
    let right = right.trim_start_matches('0');
    let left = if left.is_empty() { "0" } else { left };
    let right = if right.is_empty() { "0" } else { right };

    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

#[derive(Debug)]
enum VersionToken {
    Number(String),
    Text(String),
}

fn tokenize_version(value: &str) -> Vec<VersionToken> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut current_is_number = None::<bool>;

    for ch in value.chars() {
        let is_number = ch.is_ascii_digit();
        match current_is_number {
            Some(existing) if existing == is_number => current.push(ch),
            Some(existing) => {
                push_version_token(&mut result, std::mem::take(&mut current), existing);
                current.push(ch);
                current_is_number = Some(is_number);
            }
            None => {
                current.push(ch);
                current_is_number = Some(is_number);
            }
        }
    }

    if let Some(is_number) = current_is_number {
        push_version_token(&mut result, current, is_number);
    }

    result
}

fn push_version_token(tokens: &mut Vec<VersionToken>, value: String, is_number: bool) {
    if value.is_empty() {
        return;
    }

    if is_number {
        tokens.push(VersionToken::Number(value));
    } else {
        tokens.push(VersionToken::Text(value));
    }
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
                "WinGetUtil packaging did not emit versionData.mszyml for changed package {package_id}"
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

fn commit_output_tree(
    stage_root: &Path,
    out_root: &Path,
    changed_manifest_relpaths: &BTreeSet<String>,
    deleted_manifest_relpaths: &BTreeSet<String>,
    staged_package_files: &HashMap<String, StagedPackageFile>,
    previous_packages: &HashMap<String, StoredPackage>,
    final_packages: &HashMap<String, StoredPackage>,
) -> Result<()> {
    fs::create_dir_all(out_root)
        .with_context(|| format!("failed to create {}", out_root.display()))?;

    for relpath in changed_manifest_relpaths {
        copy_from_stage(stage_root, out_root, relpath)?;
    }

    for staged in staged_package_files.values() {
        copy_from_stage(stage_root, out_root, &staged.relpath)?;
    }

    copy_from_stage(stage_root, out_root, "source2.msix")?;

    let mut deleted_paths = deleted_manifest_relpaths.clone();
    for (package_id, old_pkg) in previous_packages {
        if !final_packages.contains_key(package_id) {
            deleted_paths.insert(old_pkg.version_data_relpath.clone());
        }
    }
    for (package_id, new_pkg) in final_packages {
        if let Some(old_pkg) = previous_packages.get(package_id)
            && old_pkg.version_data_relpath != new_pkg.version_data_relpath
        {
            deleted_paths.insert(old_pkg.version_data_relpath.clone());
        }
    }

    for relpath in deleted_paths {
        let target = out_root.join(&relpath);
        if target.is_file() {
            fs::remove_file(&target)
                .with_context(|| format!("failed to delete {}", target.display()))?;
        }
    }

    Ok(())
}

fn copy_from_stage(stage_root: &Path, out_root: &Path, relpath: &str) -> Result<()> {
    let source = stage_root.join(relpath);
    let target = out_root.join(relpath);
    ensure!(
        source.is_file(),
        "staged file is missing: {}",
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
    source2_hash: Vec<u8>,
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
        relpath: "source2.msix".to_string(),
        kind: "catalog".to_string(),
        owner_package_id: None,
        sha256: source2_hash,
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
    Ok(duration.as_nanos() as i64)
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
    use crate::adapter::windows_build_dependencies_available;

    #[test]
    fn compare_package_versions_prefers_higher_numeric_versions() {
        assert!(compare_package_versions("3.16.2", "3.15.2").is_gt());
        assert!(
            compare_package_versions("11.00.26100.1 (WinBuild.160101.0800)", "2.2.2.1").is_gt()
        );
        assert!(compare_package_versions("3.10", "3.2").is_gt());
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
        let workspace_root = resolve_workspace_root(Some(&repo)).unwrap();
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

        let args = crate::BuildArgs {
            repo,
            state: state.clone(),
            out: out.clone(),
            format: crate::CatalogFormat::V2,
        };

        run_build(args, crate::i18n::Messages::new("en")).unwrap();

        assert!(out.join("source2.msix").is_file());
        assert!(out.join("packages").is_dir());
        assert!(out.join("manifests").is_dir());
        assert!(state.join("writer").join("mutable-v2.db").is_file());
    }
}
