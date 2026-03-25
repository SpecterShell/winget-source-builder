use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
const WINGETUTIL_DLL_FILE_NAME: &str = "WinGetUtil.dll";
const PACKAGING_MSIX_RELATIVE_DIR: &str = "packaging/msix";
const APPX_MANIFEST_RELATIVE_PATH: &str = "packaging/msix/AppxManifest.xml";
const PACKAGE_OUTPUT_NAME: &str = "source2.msix";
const MISSING_PACKAGE_HRESULT: u32 = 0x8A15004D;
const V2_MAJOR_VERSION: u32 = 2;
const V2_MINOR_VERSION: u32 = 0;

#[derive(Debug, Clone)]
pub struct AdapterRequest {
    pub mutable_db_path: String,
    pub candidate_db_path: String,
    pub publish_db_path: String,
    pub stage_root: String,
    pub package_update_tracking_base_time: i64,
    pub operations: Vec<AdapterOperation>,
}

#[derive(Debug, Clone)]
pub struct AdapterOperation {
    pub kind: String,
    pub manifest_path: String,
    pub relative_path: String,
}

pub fn run_adapter(
    workspace_root: &Path,
    request: &AdapterRequest,
    stage_root: &Path,
) -> Result<()> {
    #[cfg(not(windows))]
    {
        let _ = (workspace_root, request, stage_root);
        bail!("WinGetUtil integration only runs on Windows")
    }

    #[cfg(windows)]
    {
        let winget_util_path = resolve_existing_win_get_util_path().ok_or_else(|| {
            anyhow!(
                "WinGetUtil.dll was not found next to the executable. Build the project on Windows so build.rs can provision it."
            )
        })?;
        let msix_resources_root = resolve_msix_resources_root(workspace_root)?;
        let writer = windows::WinGetWriter::load(&winget_util_path)?;
        writer.run(request, stage_root, &msix_resources_root)
    }
}

pub fn absolute_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .to_string()
}

pub fn resolve_workspace_root(repo_root_hint: Option<&Path>) -> Result<PathBuf> {
    if let Ok(override_root) = env::var("WINGET_SOURCE_BUILDER_WORKSPACE_ROOT") {
        return Ok(normalize_path(PathBuf::from(override_root)));
    }

    let candidates = workspace_root_candidates(repo_root_hint)?;
    for candidate in &candidates {
        if looks_like_packaging_root(candidate) {
            return Ok(candidate.clone());
        }
    }

    for candidate in candidates {
        if looks_like_workspace_root(&candidate) {
            return Ok(candidate);
        }
    }

    bail!(
        "failed to locate the workspace root; set WINGET_SOURCE_BUILDER_WORKSPACE_ROOT or keep packaging/msix next to the source repository"
    )
}

#[cfg(test)]
pub fn windows_build_dependencies_available(workspace_root: &Path) -> bool {
    let _ = workspace_root;
    cfg!(windows)
        && resolve_existing_win_get_util_path().is_some()
        && resolve_makeappx_path().is_some()
}

fn workspace_root_candidates(repo_root_hint: Option<&Path>) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();

    if let Some(repo_root_hint) = repo_root_hint {
        candidates.extend(ancestor_chain(repo_root_hint));
    }

    if let Ok(current_exe) = env::current_exe() {
        candidates.extend(ancestor_chain(&current_exe));
    }

    if let Ok(current_dir) = env::current_dir() {
        candidates.extend(ancestor_chain(&current_dir));
    }

    candidates.extend(ancestor_chain(Path::new(env!("CARGO_MANIFEST_DIR"))));
    dedupe_paths(candidates)
}

fn ancestor_chain(path: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let start = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    };

    let mut current = Some(start);
    while let Some(path) = current {
        result.push(path.clone());
        current = path.parent().map(Path::to_path_buf);
    }

    result
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut seen = std::collections::HashSet::<PathBuf>::new();
    let mut result = Vec::new();
    for path in paths {
        let canonical = normalize_path(path.canonicalize().unwrap_or(path.clone()));
        if seen.insert(canonical.clone()) {
            result.push(canonical);
        }
    }
    Ok(result)
}

fn looks_like_packaging_root(workspace_root: &Path) -> bool {
    workspace_root.join(APPX_MANIFEST_RELATIVE_PATH).is_file()
}

fn looks_like_workspace_root(workspace_root: &Path) -> bool {
    workspace_root.join("Cargo.toml").is_file()
        || workspace_root.join(PACKAGING_MSIX_RELATIVE_DIR).is_dir()
}

fn resolve_existing_win_get_util_path() -> Option<PathBuf> {
    let current_exe = env::current_exe().ok()?;
    let executable_dir = current_exe.parent()?;
    let candidate = executable_dir.join(WINGETUTIL_DLL_FILE_NAME);
    candidate.is_file().then_some(candidate)
}

fn resolve_msix_resources_root(workspace_root: &Path) -> Result<PathBuf> {
    let resource_root = workspace_root.join(PACKAGING_MSIX_RELATIVE_DIR);
    let manifest_path = workspace_root.join(APPX_MANIFEST_RELATIVE_PATH);

    if !manifest_path.is_file() {
        bail!(
            "MSIX AppxManifest.xml was not found at {}",
            manifest_path.display()
        );
    }

    Ok(resource_root)
}

fn resolve_makeappx_path() -> Option<PathBuf> {
    if let Ok(env_override) = env::var("MAKEAPPX_EXE") {
        let env_override = PathBuf::from(env_override);
        if env_override.is_file() {
            return Some(env_override);
        }
    }

    let program_files_x86 = env::var_os("ProgramFiles(x86)")?;
    let kits_root = PathBuf::from(program_files_x86)
        .join("Windows Kits")
        .join("10")
        .join("bin");

    if !kits_root.is_dir() {
        return None;
    }

    let mut candidates = Vec::new();
    for entry in walkdir::WalkDir::new(kits_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) != Some("makeappx.exe") {
            continue;
        }
        if !path.to_string_lossy().contains("\\x64\\") {
            continue;
        }
        candidates.push(path.to_path_buf());
    }

    candidates.sort_by(|left, right| right.cmp(left));
    candidates.into_iter().next()
}

fn normalize_path(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }

    let raw = path.to_string_lossy();
    if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        return PathBuf::from(stripped);
    }

    path
}

#[cfg(windows)]
mod windows {
    use std::ffi::{OsStr, c_void};
    use std::fs;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::ptr;
    use std::rc::Rc;

    use anyhow::{Context, Result, anyhow, bail, ensure};
    use libloading::Library;
    use log::{debug, warn};
    use walkdir::WalkDir;

    use super::{
        AdapterOperation, AdapterRequest, MISSING_PACKAGE_HRESULT, PACKAGE_OUTPUT_NAME,
        V2_MAJOR_VERSION, V2_MINOR_VERSION, resolve_makeappx_path,
    };

    type IndexHandle = *mut c_void;
    type HResult = i32;

    type CreateFn = unsafe extern "system" fn(*const u16, u32, u32, *mut IndexHandle) -> HResult;
    type OpenFn = unsafe extern "system" fn(*const u16, *mut IndexHandle) -> HResult;
    type CloseFn = unsafe extern "system" fn(IndexHandle) -> HResult;
    type SetPropertyFn = unsafe extern "system" fn(IndexHandle, u32, *const u16) -> HResult;
    type AddManifestFn = unsafe extern "system" fn(IndexHandle, *const u16, *const u16) -> HResult;
    type RemoveManifestFn =
        unsafe extern "system" fn(IndexHandle, *const u16, *const u16) -> HResult;
    type PrepareForPackagingFn = unsafe extern "system" fn(IndexHandle) -> HResult;

    const SQLITE_INDEX_PROPERTY_PACKAGE_UPDATE_TRACKING_BASE_TIME: u32 = 0;
    const SQLITE_INDEX_PROPERTY_INTERMEDIATE_FILE_OUTPUT_PATH: u32 = 1;

    #[derive(Clone)]
    pub(super) struct WinGetWriter {
        library: Rc<WinGetLibrary>,
    }

    impl WinGetWriter {
        pub(super) fn load(dll_path: &Path) -> Result<Self> {
            Ok(Self {
                library: Rc::new(WinGetLibrary::load(dll_path)?),
            })
        }

        pub(super) fn run(
            &self,
            request: &AdapterRequest,
            stage_root: &Path,
            msix_resources_root: &Path,
        ) -> Result<()> {
            let mutable_db_path = PathBuf::from(&request.mutable_db_path);
            let candidate_db_path = PathBuf::from(&request.candidate_db_path);
            let publish_db_path = PathBuf::from(&request.publish_db_path);

            if let Some(parent) = candidate_db_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            if let Some(parent) = publish_db_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::create_dir_all(stage_root)
                .with_context(|| format!("failed to create {}", stage_root.display()))?;

            if candidate_db_path.exists() {
                fs::remove_file(&candidate_db_path)
                    .with_context(|| format!("failed to remove {}", candidate_db_path.display()))?;
            }

            if mutable_db_path.is_file() {
                fs::copy(&mutable_db_path, &candidate_db_path).with_context(|| {
                    format!(
                        "failed to copy mutable db {} to {}",
                        mutable_db_path.display(),
                        candidate_db_path.display()
                    )
                })?;
            }

            let mut candidate = if candidate_db_path.is_file() {
                WinGetIndex::open(self.library.clone(), &candidate_db_path)?
            } else {
                WinGetIndex::create_v2(self.library.clone(), &candidate_db_path)?
            };

            self.apply_operations(&mut candidate, &request.operations)?;
            drop(candidate);

            if publish_db_path.exists() {
                fs::remove_file(&publish_db_path)
                    .with_context(|| format!("failed to remove {}", publish_db_path.display()))?;
            }

            fs::copy(&candidate_db_path, &publish_db_path).with_context(|| {
                format!(
                    "failed to copy candidate db {} to {}",
                    candidate_db_path.display(),
                    publish_db_path.display()
                )
            })?;

            let mut publish = WinGetIndex::open(self.library.clone(), &publish_db_path)?;
            let tracking_base_time = if request.package_update_tracking_base_time <= 0 {
                "0".to_string()
            } else {
                request.package_update_tracking_base_time.to_string()
            };
            publish.set_property(
                SQLITE_INDEX_PROPERTY_PACKAGE_UPDATE_TRACKING_BASE_TIME,
                &tracking_base_time,
            )?;
            publish.set_property(
                SQLITE_INDEX_PROPERTY_INTERMEDIATE_FILE_OUTPUT_PATH,
                &request.stage_root,
            )?;
            publish.prepare_for_packaging()?;
            drop(publish);

            package_source_msix(stage_root, &publish_db_path, msix_resources_root)?;
            Ok(())
        }

        fn apply_operations(
            &self,
            candidate: &mut WinGetIndex,
            operations: &[AdapterOperation],
        ) -> Result<()> {
            let mut pending = operations
                .iter()
                .cloned()
                .enumerate()
                .collect::<Vec<(usize, AdapterOperation)>>();
            let mut pass = 0usize;

            while !pending.is_empty() {
                pass += 1;
                let mut deferred = Vec::<(usize, AdapterOperation)>::new();
                let mut made_progress = false;

                for (index, operation) in pending {
                    let result = match operation.kind.as_str() {
                        "add" => candidate.add_manifest(
                            Path::new(&operation.manifest_path),
                            &operation.relative_path,
                        ),
                        "remove" => candidate.remove_manifest(
                            Path::new(&operation.manifest_path),
                            &operation.relative_path,
                        ),
                        kind => bail!("unsupported operation kind: {kind}"),
                    };

                    match result {
                        Ok(()) => {
                            made_progress = true;
                        }
                        Err(error)
                            if operation.kind == "add"
                                && error.downcast_ref::<HResultError>().is_some_and(|inner| {
                                    inner.hresult as u32 == MISSING_PACKAGE_HRESULT
                                }) =>
                        {
                            debug!(
                                "deferring WinGet add for {} until dependency ordering settles",
                                operation.relative_path
                            );
                            deferred.push((index, operation));
                        }
                        Err(error) => {
                            return Err(error).with_context(|| {
                                format!(
                                    "WinGet operation failed at index {index}: {} {} ({})",
                                    operation.kind,
                                    operation.relative_path,
                                    operation.manifest_path
                                )
                            });
                        }
                    }
                }

                if deferred.is_empty() {
                    break;
                }

                if !made_progress {
                    let (index, operation) = &deferred[0];
                    bail!(
                        "could not resolve package dependency ordering after {pass} pass(es); first pending add is at index {index}: {} ({})",
                        operation.relative_path,
                        operation.manifest_path
                    );
                }

                pending = deferred;
            }

            Ok(())
        }
    }

    struct WinGetLibrary {
        _library: Library,
        create: CreateFn,
        open: OpenFn,
        close: CloseFn,
        set_property: SetPropertyFn,
        add_manifest: AddManifestFn,
        remove_manifest: RemoveManifestFn,
        prepare_for_packaging: PrepareForPackagingFn,
    }

    impl WinGetLibrary {
        fn load(dll_path: &Path) -> Result<Self> {
            let library = unsafe { Library::new(dll_path) }
                .with_context(|| format!("failed to load {}", dll_path.display()))?;

            unsafe {
                let create = *library
                    .get::<CreateFn>(b"WinGetSQLiteIndexCreate\0")
                    .context("failed to resolve WinGetSQLiteIndexCreate")?;
                let open = *library
                    .get::<OpenFn>(b"WinGetSQLiteIndexOpen\0")
                    .context("failed to resolve WinGetSQLiteIndexOpen")?;
                let close = *library
                    .get::<CloseFn>(b"WinGetSQLiteIndexClose\0")
                    .context("failed to resolve WinGetSQLiteIndexClose")?;
                let set_property = *library
                    .get::<SetPropertyFn>(b"WinGetSQLiteIndexSetProperty\0")
                    .context("failed to resolve WinGetSQLiteIndexSetProperty")?;
                let add_manifest = *library
                    .get::<AddManifestFn>(b"WinGetSQLiteIndexAddManifest\0")
                    .context("failed to resolve WinGetSQLiteIndexAddManifest")?;
                let remove_manifest = *library
                    .get::<RemoveManifestFn>(b"WinGetSQLiteIndexRemoveManifest\0")
                    .context("failed to resolve WinGetSQLiteIndexRemoveManifest")?;
                let prepare_for_packaging = *library
                    .get::<PrepareForPackagingFn>(b"WinGetSQLiteIndexPrepareForPackaging\0")
                    .context("failed to resolve WinGetSQLiteIndexPrepareForPackaging")?;

                Ok(Self {
                    _library: library,
                    create,
                    open,
                    close,
                    set_property,
                    add_manifest,
                    remove_manifest,
                    prepare_for_packaging,
                })
            }
        }
    }

    struct WinGetIndex {
        library: Rc<WinGetLibrary>,
        handle: IndexHandle,
    }

    impl WinGetIndex {
        fn create_v2(library: Rc<WinGetLibrary>, db_path: &Path) -> Result<Self> {
            let mut handle = ptr::null_mut();
            let db_path = to_wide(db_path.as_os_str());
            let hr = unsafe {
                (library.create)(
                    db_path.as_ptr(),
                    V2_MAJOR_VERSION,
                    V2_MINOR_VERSION,
                    &mut handle,
                )
            };
            check_hresult(hr, || {
                format!("failed to create {}", db_path_display(db_path.as_ptr()))
            })?;
            Ok(Self { library, handle })
        }

        fn open(library: Rc<WinGetLibrary>, db_path: &Path) -> Result<Self> {
            let mut handle = ptr::null_mut();
            let db_path_wide = to_wide(db_path.as_os_str());
            let hr = unsafe { (library.open)(db_path_wide.as_ptr(), &mut handle) };
            check_hresult(hr, || format!("failed to open {}", db_path.display()))?;
            Ok(Self { library, handle })
        }

        fn set_property(&mut self, property: u32, value: &str) -> Result<()> {
            let value = to_wide(OsStr::new(value));
            let hr = unsafe { (self.library.set_property)(self.handle, property, value.as_ptr()) };
            check_hresult(hr, || format!("failed to set WinGet property {property}"))
        }

        fn add_manifest(&mut self, manifest_path: &Path, relative_path: &str) -> Result<()> {
            let manifest_path_wide = to_wide(manifest_path.as_os_str());
            let relative_path_wide = to_wide(OsStr::new(relative_path));
            let hr = unsafe {
                (self.library.add_manifest)(
                    self.handle,
                    manifest_path_wide.as_ptr(),
                    relative_path_wide.as_ptr(),
                )
            };
            check_hresult(hr, || {
                format!(
                    "failed to add manifest {} ({relative_path})",
                    manifest_path.display()
                )
            })
        }

        fn remove_manifest(&mut self, manifest_path: &Path, relative_path: &str) -> Result<()> {
            let manifest_path_wide = to_wide(manifest_path.as_os_str());
            let relative_path_wide = to_wide(OsStr::new(relative_path));
            let hr = unsafe {
                (self.library.remove_manifest)(
                    self.handle,
                    manifest_path_wide.as_ptr(),
                    relative_path_wide.as_ptr(),
                )
            };
            check_hresult(hr, || {
                format!(
                    "failed to remove manifest {} ({relative_path})",
                    manifest_path.display()
                )
            })
        }

        fn prepare_for_packaging(&mut self) -> Result<()> {
            let hr = unsafe { (self.library.prepare_for_packaging)(self.handle) };
            check_hresult(hr, || "failed to prepare index for packaging".to_string())
        }
    }

    impl Drop for WinGetIndex {
        fn drop(&mut self) {
            if self.handle.is_null() {
                return;
            }

            let hr = unsafe { (self.library.close)(self.handle) };
            if hr < 0 {
                warn!(
                    "WinGetSQLiteIndexClose failed with HRESULT 0x{:08X}",
                    hr as u32
                );
            }
            self.handle = ptr::null_mut();
        }
    }

    #[derive(Debug)]
    struct HResultError {
        context: String,
        hresult: HResult,
    }

    impl std::fmt::Display for HResultError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "{} (HRESULT 0x{:08X})",
                self.context, self.hresult as u32
            )
        }
    }

    impl std::error::Error for HResultError {}

    fn check_hresult(hr: HResult, context: impl FnOnce() -> String) -> Result<()> {
        if hr < 0 {
            return Err(HResultError {
                context: context(),
                hresult: hr,
            }
            .into());
        }

        Ok(())
    }

    fn to_wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn db_path_display(value: *const u16) -> String {
        if value.is_null() {
            return String::new();
        }

        let mut len = 0usize;
        unsafe {
            while *value.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(value, len))
        }
    }

    fn package_source_msix(
        stage_root: &Path,
        publish_db_path: &Path,
        msix_resources_root: &Path,
    ) -> Result<()> {
        let temp_dir = stage_root.join("_msix");
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir)
                .with_context(|| format!("failed to remove {}", temp_dir.display()))?;
        }

        let source_manifest_path = msix_resources_root.join("AppxManifest.xml");
        let source_assets_dir = msix_resources_root.join("Assets");
        ensure!(
            source_manifest_path.is_file(),
            "MSIX AppxManifest.xml was not found at {}",
            source_manifest_path.display()
        );
        ensure!(
            source_assets_dir.is_dir(),
            "MSIX assets directory was not found at {}",
            source_assets_dir.display()
        );

        let assets_dir = temp_dir.join("Assets");
        fs::create_dir_all(&assets_dir)
            .with_context(|| format!("failed to create {}", assets_dir.display()))?;

        let appx_manifest_path = temp_dir.join("AppxManifest.xml");
        fs::copy(&source_manifest_path, &appx_manifest_path).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source_manifest_path.display(),
                appx_manifest_path.display()
            )
        })?;

        let mut mapping_lines = vec![
            "[Files]".to_string(),
            format!("\"{}\" \"Public\\index.db\"", publish_db_path.display()),
            format!("\"{}\" \"AppxManifest.xml\"", appx_manifest_path.display()),
        ];

        let mut resource_paths = WalkDir::new(&source_assets_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.into_path())
            .collect::<Vec<_>>();
        resource_paths.sort();

        for source_asset_path in resource_paths {
            let relative_path = source_asset_path
                .strip_prefix(msix_resources_root)
                .with_context(|| {
                    format!(
                        "failed to derive relative path for {} within {}",
                        source_asset_path.display(),
                        msix_resources_root.display()
                    )
                })?;
            let staged_asset_path = temp_dir.join(relative_path);
            if let Some(parent) = staged_asset_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::copy(&source_asset_path, &staged_asset_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_asset_path.display(),
                    staged_asset_path.display()
                )
            })?;
            mapping_lines.push(format!(
                "\"{}\" \"{}\"",
                staged_asset_path.display(),
                relative_path.display().to_string().replace('/', "\\")
            ));
        }

        let mapping_file = temp_dir.join("MappingFile.txt");
        let mapping_contents = mapping_lines.join("\n") + "\n";
        fs::write(&mapping_file, mapping_contents.as_bytes())
            .with_context(|| format!("failed to write {}", mapping_file.display()))?;

        let output_package = stage_root.join(PACKAGE_OUTPUT_NAME);
        if output_package.exists() {
            fs::remove_file(&output_package)
                .with_context(|| format!("failed to remove {}", output_package.display()))?;
        }

        let makeappx = resolve_makeappx_path()
            .ok_or_else(|| anyhow!("makeappx.exe was not found in the Windows SDK"))?;
        let output = Command::new(&makeappx)
            .arg("pack")
            .arg("/o")
            .arg("/nv")
            .arg("/f")
            .arg(&mapping_file)
            .arg("/p")
            .arg(&output_package)
            .output()
            .with_context(|| format!("failed to start {}", makeappx.display()))?;

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let details = [stdout, stderr]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "makeappx failed for {}{}\n{}",
                output_package.display(),
                if details.is_empty() { "" } else { ":" },
                details
            );
        }

        Ok(())
    }
}
