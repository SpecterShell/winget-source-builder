use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use serde_yaml::{Mapping, Value as YamlValue};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Serialize)]
pub struct ComputedVersionSnapshot {
    pub version_dir: String,
    pub package_id: String,
    pub package_version: String,
    pub channel: String,
    pub index_projection: VersionIndexProjection,
    pub version_content_sha256: Vec<u8>,
    pub installers: Vec<InstallerRecord>,
    pub version_installer_sha256: Vec<u8>,
    pub published_manifest_sha256: Vec<u8>,
    pub published_manifest_relpath: String,
    #[serde(skip_serializing)]
    pub published_manifest_bytes: Vec<u8>,
    pub source_file_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationRequirement {
    pub package_id: String,
    pub package_version: String,
    pub channel: String,
    pub installer: InstallerRecord,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct InstallerRecord {
    pub installer_sha256: String,
    pub installer_url: Option<String>,
    pub architecture: Option<String>,
    pub installer_type: Option<String>,
    pub installer_locale: Option<String>,
    pub scope: Option<String>,
    pub package_family_name: Option<String>,
    pub product_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersionIndexProjection {
    pub package_name: String,
    pub publisher: Option<String>,
    pub moniker: Option<String>,
    pub arp_min_version: Option<String>,
    pub arp_max_version: Option<String>,
    pub tags: Vec<String>,
    pub commands: Vec<String>,
    pub package_family_names: Vec<String>,
    pub product_codes: Vec<String>,
    pub upgrade_codes: Vec<String>,
    pub normalized_names: Vec<String>,
    pub normalized_publishers: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ManifestWarning {
    NumericPackageVersion {
        manifest_path: PathBuf,
        package_version: String,
    },
}

#[derive(Debug, Clone)]
pub struct ComputedVersionResult {
    pub snapshot: ComputedVersionSnapshot,
    pub warnings: Vec<ManifestWarning>,
}

#[derive(Debug)]
struct SourceDoc {
    file_name: String,
    root: YamlValue,
    manifest_type: String,
    package_version: String,
    warnings: Vec<ManifestWarning>,
}

pub fn scan_root(repo_root: &Path) -> PathBuf {
    let manifests = repo_root.join("manifests");
    if manifests.is_dir() {
        manifests
    } else {
        repo_root.to_path_buf()
    }
}

#[cfg(test)]
pub fn compute_version_snapshot(
    repo_root: &Path,
    version_dir_abs: &Path,
    version_dir_rel: &str,
) -> Result<ComputedVersionSnapshot> {
    Ok(
        compute_version_snapshot_with_warnings(repo_root, version_dir_abs, version_dir_rel)?
            .snapshot,
    )
}

pub fn compute_version_snapshot_with_warnings(
    repo_root: &Path,
    version_dir_abs: &Path,
    version_dir_rel: &str,
) -> Result<ComputedVersionResult> {
    let mut files = fs::read_dir(version_dir_abs)
        .with_context(|| {
            format!(
                "failed to read version directory {}",
                version_dir_abs.display()
            )
        })?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|ty| ty.is_file()).unwrap_or(false))
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("yaml"))
        .collect::<Vec<_>>();

    files.sort();
    ensure!(
        !files.is_empty(),
        "no manifest files found in {}",
        version_dir_abs.display()
    );

    let docs = files
        .iter()
        .map(|path| read_doc(path.as_path()))
        .collect::<Result<Vec<_>>>()
        .with_context(|| format!("failed to parse {}", version_dir_abs.display()))?;
    let warnings = docs
        .iter()
        .flat_map(|doc| doc.warnings.iter().cloned())
        .collect::<Vec<_>>();

    let merged = merge_docs(docs)
        .with_context(|| format!("failed to merge manifests in {}", version_dir_abs.display()))?;
    let snapshot =
        build_snapshot_from_merged_manifest(repo_root, version_dir_rel, files.len(), merged)
            .with_context(|| format!("failed to normalize {}", version_dir_abs.display()))?;

    Ok(ComputedVersionResult { snapshot, warnings })
}

pub fn extract_display_versions_from_manifest_bytes(bytes: &[u8]) -> Result<BTreeSet<String>> {
    let root: YamlValue =
        serde_yaml::from_slice(bytes).context("failed to parse staged merged manifest YAML")?;
    extract_display_versions_from_manifest(&root)
}

pub fn retain_display_versions_in_snapshot(
    repo_root: &Path,
    snapshot: &ComputedVersionSnapshot,
    retained_display_versions: &BTreeSet<String>,
) -> Result<ComputedVersionSnapshot> {
    let mut root: YamlValue = serde_yaml::from_slice(&snapshot.published_manifest_bytes)
        .context("failed to parse staged merged manifest YAML")?;
    retain_display_versions_in_manifest(&mut root, retained_display_versions)?;
    build_snapshot_from_merged_manifest(
        repo_root,
        &snapshot.version_dir,
        snapshot.source_file_count,
        root,
    )
}

fn read_doc(path: &Path) -> Result<SourceDoc> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest file {}", path.display()))?;
    let mut root: YamlValue = serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse YAML {}", path.display()))?;
    let numeric_package_version = as_mapping(&root)?
        .get(YamlValue::String("PackageVersion".to_string()))
        .is_some_and(|value| matches!(value, YamlValue::Number(_)));
    let package_version = extract_top_level_scalar_string(&raw, "PackageVersion")
        .or_else(|| get_optional_string(&root, "PackageVersion"))
        .ok_or_else(|| anyhow!("required field PackageVersion is missing"))?;
    set_top_level_string_field(&mut root, "PackageVersion", &package_version)?;
    let manifest_type = get_required_string(&root, "ManifestType")?;
    let mut warnings = Vec::new();
    if numeric_package_version {
        warnings.push(ManifestWarning::NumericPackageVersion {
            manifest_path: path.to_path_buf(),
            package_version: package_version.clone(),
        });
    }

    Ok(SourceDoc {
        file_name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        root,
        manifest_type,
        package_version,
        warnings,
    })
}

fn merge_docs(mut docs: Vec<SourceDoc>) -> Result<YamlValue> {
    ensure!(!docs.is_empty(), "empty version directory");

    if docs.len() == 1 {
        let doc = docs.pop().unwrap();
        let manifest_type = doc.manifest_type.to_ascii_lowercase();
        if manifest_type != "merged" && manifest_type != "singleton" {
            bail!(
                "single-file manifest {} must be merged or singleton, found {}",
                doc.file_name,
                doc.manifest_type
            );
        }
        ensure_common_root_fields(&doc.root)?;
        return Ok(doc.root);
    }

    let mut package_id = None::<String>;
    let mut package_version = None::<String>;
    let mut manifest_version = None::<String>;
    let mut version_doc = None::<YamlValue>;
    let mut installer_doc = None::<YamlValue>;
    let mut default_locale_doc = None::<YamlValue>;
    let mut locale_docs = Vec::<YamlValue>::new();

    docs.sort_by(|left, right| left.file_name.cmp(&right.file_name));

    for doc in docs {
        ensure_common_root_fields(&doc.root)?;
        let local_id = get_required_string(&doc.root, "PackageIdentifier")?;
        let local_version = doc.package_version.clone();
        let local_manifest_version = get_required_string(&doc.root, "ManifestVersion")?;

        if let Some(expected) = &package_id {
            ensure!(
                expected == &local_id,
                "inconsistent PackageIdentifier in multifile manifest"
            );
        } else {
            package_id = Some(local_id);
        }

        if let Some(expected) = &package_version {
            ensure!(
                expected == &local_version,
                "inconsistent PackageVersion in multifile manifest"
            );
        } else {
            package_version = Some(local_version);
        }

        if let Some(expected) = &manifest_version {
            ensure!(
                expected == &local_manifest_version,
                "inconsistent ManifestVersion in multifile manifest"
            );
        } else {
            manifest_version = Some(local_manifest_version);
        }

        match doc.manifest_type.to_ascii_lowercase().as_str() {
            "version" => {
                ensure!(version_doc.is_none(), "duplicate version manifest");
                version_doc = Some(doc.root);
            }
            "installer" => {
                ensure!(installer_doc.is_none(), "duplicate installer manifest");
                installer_doc = Some(doc.root);
            }
            "defaultlocale" => {
                ensure!(
                    default_locale_doc.is_none(),
                    "duplicate defaultLocale manifest"
                );
                default_locale_doc = Some(doc.root);
            }
            "locale" => locale_docs.push(doc.root),
            other => bail!("unsupported multifile manifest type {other}"),
        }
    }

    ensure!(version_doc.is_some(), "missing version manifest");
    let installer_doc = installer_doc.ok_or_else(|| anyhow!("missing installer manifest"))?;
    let default_locale_doc =
        default_locale_doc.ok_or_else(|| anyhow!("missing defaultLocale manifest"))?;

    let default_locale = get_required_string(&default_locale_doc, "PackageLocale")?;
    let version_default_locale =
        get_required_string(version_doc.as_ref().unwrap(), "DefaultLocale")?;
    ensure!(
        default_locale.eq_ignore_ascii_case(&version_default_locale),
        "DefaultLocale in version manifest does not match PackageLocale in defaultLocale manifest"
    );

    let mut merged = installer_doc;
    merge_non_common_fields(&default_locale_doc, &mut merged)?;

    locale_docs.sort_by(|left, right| {
        get_optional_string(left, "PackageLocale")
            .unwrap_or_default()
            .cmp(&get_optional_string(right, "PackageLocale").unwrap_or_default())
    });

    if !locale_docs.is_empty() {
        let mut localization = Vec::new();
        for doc in locale_docs {
            let mut localization_doc = YamlValue::Mapping(Mapping::new());
            merge_non_common_fields(&doc, &mut localization_doc)?;
            localization.push(localization_doc);
        }

        let merged_map = as_mapping_mut(&mut merged)?;
        merged_map.insert(
            YamlValue::String("Localization".to_string()),
            YamlValue::Sequence(localization),
        );
    }

    as_mapping_mut(&mut merged)?.insert(
        YamlValue::String("ManifestType".to_string()),
        YamlValue::String("merged".to_string()),
    );

    Ok(merged)
}

fn ensure_common_root_fields(root: &YamlValue) -> Result<()> {
    let map = as_mapping(root)?;
    for key in [
        "PackageIdentifier",
        "PackageVersion",
        "ManifestVersion",
        "ManifestType",
    ] {
        ensure!(
            map.get(YamlValue::String(key.to_string())).is_some(),
            "required field {key} is missing"
        );
    }
    Ok(())
}

fn merge_non_common_fields(source: &YamlValue, destination: &mut YamlValue) -> Result<()> {
    let common = [
        "PackageIdentifier",
        "PackageVersion",
        "ManifestVersion",
        "ManifestType",
    ];

    let source_map = as_mapping(source)?;
    let destination_map = as_mapping_mut(destination)?;

    for (key, value) in source_map {
        let key_string = key
            .as_str()
            .ok_or_else(|| anyhow!("manifest key must be a string"))?;
        if common.contains(&key_string) {
            continue;
        }
        destination_map.insert(key.clone(), value.clone());
    }

    Ok(())
}

fn build_manifest_relpath(
    _repo_root: &Path,
    package_id: &str,
    package_version: &str,
    manifest_sha256: &[u8],
) -> String {
    let first = package_id
        .chars()
        .next()
        .unwrap_or('x')
        .to_ascii_lowercase()
        .to_string();
    let path_segments = package_id
        .split('.')
        .map(sanitize_segment)
        .collect::<Vec<_>>()
        .join("/");
    let hash_prefix = &hex::encode(manifest_sha256)[0..8];

    format!(
        "manifests/{first}/{path_segments}/{}/{}-{}.yaml",
        sanitize_segment(package_version),
        hash_prefix,
        sanitize_segment(package_id),
    )
}

fn deterministic_manifest_yaml(canonical: &JsonValue) -> Result<Vec<u8>> {
    let yaml =
        serde_yaml::to_string(canonical).context("failed to convert canonical manifest to YAML")?;
    Ok(yaml.into_bytes())
}

fn build_snapshot_from_merged_manifest(
    repo_root: &Path,
    version_dir_rel: &str,
    source_file_count: usize,
    mut merged: YamlValue,
) -> Result<ComputedVersionSnapshot> {
    let version_dir = normalize_rel(version_dir_rel);
    let expected_package_version = version_segment(&version_dir)?;

    normalize_top_level_string_fields(&mut merged, &expected_package_version)?;

    let canonical_content = canonicalize_full_manifest(&merged)?;
    let installers = build_installer_records(&merged)?;
    let index_projection = build_index_projection(&merged)?;

    let canonical_content_bytes =
        serde_json::to_vec(&canonical_content).context("failed to serialize canonical content")?;
    let installer_bytes =
        serde_json::to_vec(&installers).context("failed to serialize installer projections")?;

    let published_manifest_bytes =
        deterministic_manifest_yaml(&canonical_content).context("failed to render merged YAML")?;

    let package_id = get_required_string(&merged, "PackageIdentifier")?;
    let package_version = get_required_string(&merged, "PackageVersion")?;
    let channel = get_optional_string(&merged, "Channel").unwrap_or_default();
    let published_manifest_sha256 = sha256_bytes(&published_manifest_bytes);
    let published_manifest_relpath = build_manifest_relpath(
        repo_root,
        &package_id,
        &package_version,
        &published_manifest_sha256,
    );

    Ok(ComputedVersionSnapshot {
        version_dir,
        package_id,
        package_version,
        channel,
        index_projection,
        version_content_sha256: sha256_bytes(&canonical_content_bytes),
        installers,
        version_installer_sha256: sha256_bytes(&installer_bytes),
        published_manifest_sha256,
        published_manifest_relpath,
        published_manifest_bytes,
        source_file_count,
    })
}

fn build_index_projection(root: &YamlValue) -> Result<VersionIndexProjection> {
    let mut tags = BTreeSet::new();
    let mut commands = BTreeSet::new();
    let mut package_family_names = BTreeSet::new();
    let mut product_codes = BTreeSet::new();
    let mut upgrade_codes = BTreeSet::new();
    let mut normalized_names = BTreeSet::new();
    let mut normalized_publishers = BTreeSet::new();

    let package_name = get_optional_string(root, "PackageName").unwrap_or_default();
    let publisher = get_optional_string(root, "Publisher");
    let moniker = get_optional_string(root, "Moniker");
    let arp_min_version = get_optional_string(root, "ArpMinVersion");
    let arp_max_version = get_optional_string(root, "ArpMaxVersion");

    for tag in get_sequence_strings(root, "Tags")? {
        tags.insert(tag);
    }
    for command in get_sequence_strings(root, "Commands")? {
        commands.insert(command);
    }
    if let Some(package_family_name) = get_optional_string(root, "PackageFamilyName") {
        package_family_names.insert(package_family_name);
    }

    add_normalized_name(&mut normalized_names, &package_name, false);
    if let Some(publisher) = get_optional_string(root, "Publisher") {
        add_normalized_publisher(&mut normalized_publishers, &publisher);
    }

    for localization in get_localizations(root)? {
        if let Some(name) = get_optional_string(localization, "PackageName") {
            add_normalized_name(&mut normalized_names, &name, false);
        }
        if let Some(publisher) = get_optional_string(localization, "Publisher") {
            add_normalized_publisher(&mut normalized_publishers, &publisher);
        }
    }

    collect_projection_entry_data(
        root,
        &mut commands,
        &mut package_family_names,
        &mut product_codes,
        &mut upgrade_codes,
        &mut normalized_names,
        &mut normalized_publishers,
    );

    for installer in get_installers(root)? {
        collect_projection_entry_data(
            installer,
            &mut commands,
            &mut package_family_names,
            &mut product_codes,
            &mut upgrade_codes,
            &mut normalized_names,
            &mut normalized_publishers,
        );
    }

    Ok(VersionIndexProjection {
        package_name,
        publisher,
        moniker,
        arp_min_version,
        arp_max_version,
        tags: tags.into_iter().collect(),
        commands: commands.into_iter().collect(),
        package_family_names: package_family_names.into_iter().collect(),
        product_codes: product_codes.into_iter().collect(),
        upgrade_codes: upgrade_codes.into_iter().collect(),
        normalized_names: normalized_names.into_iter().collect(),
        normalized_publishers: normalized_publishers.into_iter().collect(),
    })
}

fn collect_projection_entry_data(
    value: &YamlValue,
    commands: &mut BTreeSet<String>,
    package_family_names: &mut BTreeSet<String>,
    product_codes: &mut BTreeSet<String>,
    upgrade_codes: &mut BTreeSet<String>,
    normalized_names: &mut BTreeSet<String>,
    normalized_publishers: &mut BTreeSet<String>,
) {
    if let Ok(sequence_commands) = get_sequence_strings(value, "Commands") {
        commands.extend(sequence_commands);
    }
    if let Some(package_family_name) = get_optional_string(value, "PackageFamilyName") {
        package_family_names.insert(package_family_name);
    }

    for entry in get_apps_and_features_entries(value) {
        if let Some(display_name) = get_optional_string(entry, "DisplayName") {
            add_normalized_name(normalized_names, &display_name, true);
        }
        if let Some(publisher) = get_optional_string(entry, "Publisher") {
            add_normalized_publisher(normalized_publishers, &publisher);
        }
        if let Some(product_code) = get_optional_string(entry, "ProductCode") {
            product_codes.insert(product_code);
        }
        if let Some(upgrade_code) = get_optional_string(entry, "UpgradeCode") {
            upgrade_codes.insert(upgrade_code);
        }
    }
}

fn get_installers(root: &YamlValue) -> Result<Vec<&YamlValue>> {
    Ok(as_mapping(root)?
        .get(YamlValue::String("Installers".to_string()))
        .and_then(YamlValue::as_sequence)
        .map(|items| items.iter().collect())
        .unwrap_or_default())
}

fn get_localizations(root: &YamlValue) -> Result<Vec<&YamlValue>> {
    Ok(as_mapping(root)?
        .get(YamlValue::String("Localization".to_string()))
        .and_then(YamlValue::as_sequence)
        .map(|items| items.iter().collect())
        .unwrap_or_default())
}

fn get_apps_and_features_entries(value: &YamlValue) -> Vec<&YamlValue> {
    value
        .as_mapping()
        .and_then(|map| map.get(YamlValue::String("AppsAndFeaturesEntries".to_string())))
        .and_then(YamlValue::as_sequence)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn get_sequence_strings(value: &YamlValue, key: &str) -> Result<Vec<String>> {
    let Some(sequence) = as_mapping(value)?
        .get(YamlValue::String(key.to_string()))
        .and_then(YamlValue::as_sequence)
    else {
        return Ok(Vec::new());
    };

    Ok(sequence
        .iter()
        .filter_map(yaml_scalar_to_string)
        .collect::<Vec<_>>())
}

fn add_normalized_name(
    out: &mut BTreeSet<String>,
    value: &str,
    include_architecture_variant: bool,
) {
    let normalized = normalize_name(value);
    if !normalized.base.is_empty() {
        out.insert(normalized.base.clone());
    }
    if include_architecture_variant && let Some(arch) = normalized.architecture {
        out.insert(format!("{}({arch})", normalized.base));
    }
}

fn add_normalized_publisher(out: &mut BTreeSet<String>, value: &str) {
    let normalized = normalize_publisher(value);
    if !normalized.is_empty() {
        out.insert(normalized);
    }
}

#[derive(Debug)]
struct NormalizedNameParts {
    base: String,
    architecture: Option<&'static str>,
}

fn normalize_name(value: &str) -> NormalizedNameParts {
    let normalized = value.nfkc().collect::<String>().to_lowercase();
    let architecture = detect_architecture(&normalized);
    let without_arch = strip_architecture_tokens(&normalized);
    let without_locale = strip_locale_tokens(&without_arch);

    NormalizedNameParts {
        base: collapse_search_text(&without_locale),
        architecture,
    }
}

fn normalize_publisher(value: &str) -> String {
    let folded = value.nfkc().collect::<String>().to_lowercase();
    let mut tokens = Vec::new();

    for token in folded
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
    {
        if !tokens.is_empty() && is_legal_entity_suffix(token) {
            break;
        }
        tokens.push(token);
    }

    collapse_search_text(&tokens.join(""))
}

fn is_legal_entity_suffix(token: &str) -> bool {
    matches!(
        token,
        "ab" | "ad"
            | "ag"
            | "aps"
            | "as"
            | "asa"
            | "bv"
            | "co"
            | "company"
            | "corp"
            | "corporation"
            | "cv"
            | "doo"
            | "ev"
            | "ges"
            | "gesmbh"
            | "gmbh"
            | "holding"
            | "holdings"
            | "inc"
            | "incorporated"
            | "kg"
            | "ks"
            | "limited"
            | "llc"
            | "lp"
            | "ltd"
            | "ltda"
            | "mbh"
            | "nv"
            | "plc"
            | "ps"
            | "pty"
            | "pvt"
            | "sa"
            | "sarl"
            | "sca"
            | "sc"
            | "sl"
            | "spa"
            | "sp"
            | "srl"
            | "sro"
            | "subsidiary"
    )
}

fn collapse_search_text(value: &str) -> String {
    value.chars().filter(|ch| ch.is_alphanumeric()).collect()
}

fn detect_architecture(value: &str) -> Option<&'static str> {
    let has_x86 = contains_architecture_token(value, &["x86", "32 bit", "32-bit"]);
    let has_x64 = contains_architecture_token(value, &["x64", "x86_64", "64 bit", "64-bit"]);

    match (has_x86, has_x64) {
        (true, false) => Some("X86"),
        (false, true) => Some("X64"),
        _ => None,
    }
}

fn contains_architecture_token(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn strip_architecture_tokens(value: &str) -> String {
    let mut result = value.to_string();
    for token in [
        "x86_64", "x64", "x86", "64-bit", "64 bit", "32-bit", "32 bit",
    ] {
        result = result.replace(token, " ");
    }
    result
}

fn strip_locale_tokens(value: &str) -> String {
    let mut tokens = Vec::new();
    for token in value.split(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')' | '[' | ']')) {
        if token.len() == 5 {
            let bytes = token.as_bytes();
            if bytes[0].is_ascii_alphabetic()
                && bytes[1].is_ascii_alphabetic()
                && bytes[2] == b'-'
                && bytes[3].is_ascii_alphabetic()
                && bytes[4].is_ascii_alphabetic()
            {
                continue;
            }
        }
        if !token.is_empty() {
            tokens.push(token);
        }
    }

    tokens.join(" ")
}

fn normalize_top_level_string_fields(root: &mut YamlValue, package_version: &str) -> Result<()> {
    set_top_level_string_field(root, "PackageVersion", package_version)?;

    for key in [
        "PackageIdentifier",
        "ManifestVersion",
        "ManifestType",
        "Channel",
        "DefaultLocale",
        "PackageLocale",
    ] {
        if let Some(value) = get_optional_string(root, key) {
            set_top_level_string_field(root, key, &value)?;
        }
    }

    Ok(())
}

fn version_segment(version_dir: &str) -> Result<String> {
    version_dir
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("version directory {version_dir} has no terminal segment"))
}

fn set_top_level_string_field(root: &mut YamlValue, key: &str, value: &str) -> Result<()> {
    as_mapping_mut(root)?.insert(
        YamlValue::String(key.to_string()),
        YamlValue::String(value.to_string()),
    );
    Ok(())
}

fn canonicalize_full_manifest(root: &YamlValue) -> Result<JsonValue> {
    canonicalize_yaml(root, &[])
}

pub fn installer_records_to_json(installers: &[InstallerRecord]) -> Result<String> {
    serde_json::to_string(installers).context("failed to serialize installer records")
}

pub fn parse_installer_records_json(json: Option<&str>) -> Result<Vec<InstallerRecord>> {
    match json {
        Some(json) if !json.trim().is_empty() => {
            serde_json::from_str(json).context("failed to deserialize installer records")
        }
        _ => Ok(Vec::new()),
    }
}

pub fn added_installers(
    previous: &[InstallerRecord],
    current: &[InstallerRecord],
) -> Vec<InstallerRecord> {
    let previous_hashes = previous
        .iter()
        .map(|installer| installer.installer_sha256.as_str())
        .collect::<BTreeSet<_>>();

    current
        .iter()
        .filter(|installer| !previous_hashes.contains(installer.installer_sha256.as_str()))
        .cloned()
        .collect()
}

fn build_installer_records(root: &YamlValue) -> Result<Vec<InstallerRecord>> {
    let root_map = as_mapping(root)?;
    let installers = root_map
        .get(YamlValue::String("Installers".to_string()))
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| anyhow!("merged manifest is missing Installers"))?;

    let mut records = installers
        .iter()
        .map(|installer| build_installer_record(root, installer))
        .collect::<Result<Vec<_>>>()?;
    records.sort();
    records.dedup();
    Ok(records)
}

fn build_installer_record(root: &YamlValue, installer: &YamlValue) -> Result<InstallerRecord> {
    let effective = effective_installer_manifest(root, installer)?;
    let canonical = canonicalize_yaml(&effective, &[])?;
    let canonical_bytes =
        serde_json::to_vec(&canonical).context("failed to serialize installer record")?;

    let product_codes = get_apps_and_features_entries(&effective)
        .into_iter()
        .filter_map(|entry| get_optional_string(entry, "ProductCode"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    Ok(InstallerRecord {
        installer_sha256: hex::encode(sha256_bytes(&canonical_bytes)),
        installer_url: get_optional_string(&effective, "InstallerUrl"),
        architecture: get_optional_string(&effective, "Architecture"),
        installer_type: get_optional_string(&effective, "InstallerType"),
        installer_locale: get_optional_string(&effective, "InstallerLocale")
            .or_else(|| get_optional_string(&effective, "PackageLocale")),
        scope: get_optional_string(&effective, "Scope"),
        package_family_name: get_optional_string(&effective, "PackageFamilyName"),
        product_codes,
    })
}

fn effective_installer_manifest(root: &YamlValue, installer: &YamlValue) -> Result<YamlValue> {
    let filtered_root = filter_for_installer_hash(root, true)?;
    let filtered_installer = filter_for_installer_hash(installer, false)?;
    let root_map = as_mapping(&filtered_root)?;
    let installer_map = as_mapping(&filtered_installer)?;

    let mut effective = Mapping::new();
    for identity_key in ["PackageIdentifier", "PackageVersion", "Channel"] {
        if let Some(value) = root_map.get(YamlValue::String(identity_key.to_string())) {
            effective.insert(YamlValue::String(identity_key.to_string()), value.clone());
        }
    }

    let mut installer_effective = Mapping::new();
    for (key, value) in root_map {
        let key_string = key
            .as_str()
            .ok_or_else(|| anyhow!("manifest key must be a string"))?;
        if matches!(
            key_string,
            "PackageIdentifier" | "PackageVersion" | "Channel" | "Installers"
        ) {
            continue;
        }
        installer_effective.insert(key.clone(), value.clone());
    }

    for (key, value) in installer_map {
        installer_effective.insert(key.clone(), value.clone());
    }

    effective.insert(
        YamlValue::String("Installer".to_string()),
        YamlValue::Mapping(installer_effective),
    );
    Ok(YamlValue::Mapping(effective))
}

fn canonicalize_yaml(value: &YamlValue, path: &[String]) -> Result<JsonValue> {
    match value {
        YamlValue::Null => Ok(JsonValue::Null),
        YamlValue::Bool(value) => Ok(JsonValue::Bool(*value)),
        YamlValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(JsonValue::Number(serde_json::Number::from(value)))
            } else if let Some(value) = number.as_u64() {
                Ok(JsonValue::Number(serde_json::Number::from(value)))
            } else if let Some(value) = number.as_f64() {
                let json_number = serde_json::Number::from_f64(value)
                    .ok_or_else(|| anyhow!("invalid number value"))?;
                Ok(JsonValue::Number(json_number))
            } else {
                bail!("unsupported YAML number value");
            }
        }
        YamlValue::String(value) => Ok(JsonValue::String(value.clone())),
        YamlValue::Sequence(items) => {
            let key = path.last().map(String::as_str).unwrap_or_default();
            let mut converted = items
                .iter()
                .map(|item| canonicalize_yaml(item, path))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .filter(|value| !is_omittable_json(value))
                .collect::<Vec<_>>();

            if key == "Localization" {
                converted.sort_by(|left, right| {
                    localization_sort_key(left).cmp(&localization_sort_key(right))
                });
            } else if should_sort_sequence(key) {
                converted.sort_by(|left, right| {
                    canonical_sort_key(left).cmp(&canonical_sort_key(right))
                });
            }

            Ok(JsonValue::Array(converted))
        }
        YamlValue::Mapping(mapping) => {
            let mut ordered = BTreeMap::<String, JsonValue>::new();
            for (key, value) in mapping {
                let key_string = key
                    .as_str()
                    .ok_or_else(|| anyhow!("manifest key must be a string"))?
                    .to_string();
                let mut child_path = path.to_vec();
                child_path.push(key_string.clone());
                let canonical_child = canonicalize_yaml(value, &child_path)?;
                if is_omittable_json(&canonical_child) {
                    continue;
                }
                ordered.insert(key_string, canonical_child);
            }

            let mut json_map = JsonMap::new();
            for (key, value) in ordered {
                json_map.insert(key, value);
            }
            Ok(JsonValue::Object(json_map))
        }
        YamlValue::Tagged(tagged) => canonicalize_yaml(&tagged.value, path),
    }
}

fn is_omittable_json(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => true,
        JsonValue::Array(items) => items.is_empty(),
        JsonValue::Object(map) => map.is_empty(),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => false,
    }
}

fn extract_display_versions_from_manifest(root: &YamlValue) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    collect_display_versions(root, &mut result);
    let installers = as_mapping(root)?
        .get(YamlValue::String("Installers".to_string()))
        .and_then(YamlValue::as_sequence)
        .into_iter()
        .flatten();

    for installer in installers {
        collect_display_versions(installer, &mut result);
    }

    Ok(result)
}

fn retain_display_versions_in_manifest(
    root: &mut YamlValue,
    retained_display_versions: &BTreeSet<String>,
) -> Result<()> {
    retain_display_versions(root, retained_display_versions);

    let installers = as_mapping_mut(root)?
        .get_mut(YamlValue::String("Installers".to_string()))
        .and_then(YamlValue::as_sequence_mut)
        .ok_or_else(|| anyhow!("merged manifest is missing Installers"))?;

    for installer in installers {
        retain_display_versions(installer, retained_display_versions);
    }

    Ok(())
}

fn collect_display_versions(value: &YamlValue, result: &mut BTreeSet<String>) {
    let apps_and_features = as_mapping(value)
        .ok()
        .and_then(|map| map.get(YamlValue::String("AppsAndFeaturesEntries".to_string())))
        .and_then(YamlValue::as_sequence)
        .into_iter()
        .flatten();

    for entry in apps_and_features {
        if let Some(display_version) = get_optional_string(entry, "DisplayVersion") {
            result.insert(display_version);
        }
    }
}

fn retain_display_versions(value: &mut YamlValue, retained_display_versions: &BTreeSet<String>) {
    let Some(apps_and_features) = value
        .as_mapping_mut()
        .and_then(|map| map.get_mut(YamlValue::String("AppsAndFeaturesEntries".to_string())))
        .and_then(YamlValue::as_sequence_mut)
    else {
        return;
    };

    for entry in apps_and_features {
        if let Some(map) = entry.as_mapping_mut() {
            let key = YamlValue::String("DisplayVersion".to_string());
            let display_version = map.get(&key).and_then(yaml_scalar_to_string);
            if display_version
                .as_ref()
                .is_none_or(|value| !retained_display_versions.contains(value))
            {
                map.remove(key);
            }
        }
    }
}

fn filter_for_installer_hash(value: &YamlValue, is_root: bool) -> Result<YamlValue> {
    match value {
        YamlValue::Mapping(mapping) => {
            let mut result = Mapping::new();
            for (key, child) in mapping {
                let key_string = key
                    .as_str()
                    .ok_or_else(|| anyhow!("manifest key must be a string"))?;

                if should_exclude_for_installer_hash(key_string, is_root) {
                    continue;
                }

                result.insert(key.clone(), filter_for_installer_hash(child, false)?);
            }
            Ok(YamlValue::Mapping(result))
        }
        YamlValue::Sequence(values) => Ok(YamlValue::Sequence(
            values
                .iter()
                .map(|value| filter_for_installer_hash(value, false))
                .collect::<Result<Vec<_>>>()?,
        )),
        YamlValue::Tagged(tagged) => filter_for_installer_hash(&tagged.value, is_root),
        other => Ok(other.clone()),
    }
}

fn should_sort_sequence(key: &str) -> bool {
    matches!(
        key,
        "Tags"
            | "Capabilities"
            | "RestrictedCapabilities"
            | "Platform"
            | "InstallModes"
            | "UnsupportedArguments"
            | "UnsupportedOSArchitectures"
            | "Commands"
            | "Protocols"
            | "FileExtensions"
            | "AllowedMarkets"
            | "ExcludedMarkets"
            | "PackageDependencies"
            | "WindowsFeatures"
            | "ExternalDependencies"
    )
}

fn should_exclude_for_installer_hash(key: &str, is_root: bool) -> bool {
    if matches!(key, "Commands" | "Protocols" | "FileExtensions") {
        return true;
    }

    if !is_root {
        return false;
    }

    matches!(
        key,
        "PackageLocale"
            | "DefaultLocale"
            | "Publisher"
            | "PublisherUrl"
            | "PublisherSupportUrl"
            | "PrivacyUrl"
            | "Author"
            | "PackageName"
            | "PackageUrl"
            | "License"
            | "LicenseUrl"
            | "Copyright"
            | "CopyrightUrl"
            | "ShortDescription"
            | "Description"
            | "Tags"
            | "Agreements"
            | "Documentations"
            | "ReleaseNotes"
            | "ReleaseNotesUrl"
            | "PurchaseUrl"
            | "InstallationNotes"
            | "Icons"
            | "Localization"
            | "ManifestVersion"
            | "ManifestType"
            | "Moniker"
            | "ReleaseDate"
            | "DisplayInstallWarnings"
    )
}

fn localization_sort_key(value: &JsonValue) -> String {
    value
        .as_object()
        .and_then(|map| map.get("PackageLocale"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn canonical_sort_key(value: &JsonValue) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn get_required_string(value: &YamlValue, key: &str) -> Result<String> {
    get_optional_string(value, key).ok_or_else(|| anyhow!("required field {key} is missing"))
}

fn get_optional_string(value: &YamlValue, key: &str) -> Option<String> {
    as_mapping(value)
        .ok()
        .and_then(|map| map.get(YamlValue::String(key.to_string())))
        .and_then(yaml_scalar_to_string)
}

fn yaml_scalar_to_string(value: &YamlValue) -> Option<String> {
    match value {
        YamlValue::String(value) => Some(value.clone()),
        YamlValue::Number(value) => Some(value.to_string()),
        YamlValue::Bool(value) => Some(value.to_string()),
        YamlValue::Tagged(tagged) => yaml_scalar_to_string(&tagged.value),
        YamlValue::Null | YamlValue::Sequence(_) | YamlValue::Mapping(_) => None,
    }
}

fn extract_top_level_scalar_string(raw: &str, key: &str) -> Option<String> {
    for line in raw.lines() {
        let line = line.trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }

        let (candidate_key, raw_value) = line.split_once(':')?;
        if candidate_key.trim() != key {
            continue;
        }

        let value = strip_inline_yaml_comment(raw_value).trim();
        if value.is_empty() || matches!(value.chars().next(), Some('|') | Some('>')) {
            return None;
        }

        return if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            serde_yaml::from_str::<String>(value)
                .ok()
                .or_else(|| Some(value[1..value.len() - 1].to_string()))
        } else {
            Some(value.to_string())
        };
    }

    None
}

fn strip_inline_yaml_comment(raw_value: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for (index, ch) in raw_value.char_indices() {
        match ch {
            '\\' if in_double => {
                escaped = !escaped;
            }
            '\'' if !in_double && !escaped => in_single = !in_single,
            '"' if !in_single && !escaped => in_double = !in_double,
            '#' if !in_single && !in_double && !escaped => return &raw_value[..index],
            _ => escaped = false,
        }
    }

    raw_value
}

fn as_mapping(value: &YamlValue) -> Result<&Mapping> {
    value
        .as_mapping()
        .ok_or_else(|| anyhow!("manifest root must be a mapping"))
}

fn as_mapping_mut(value: &mut YamlValue) -> Result<&mut Mapping> {
    value
        .as_mapping_mut()
        .ok_or_else(|| anyhow!("manifest root must be a mapping"))
}

pub fn sha256_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().to_vec()
}

pub fn normalize_rel(path: &str) -> String {
    path.replace('\\', "/")
}

fn sanitize_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn parse_yaml(input: &str) -> YamlValue {
        serde_yaml::from_str(input).unwrap()
    }

    #[test]
    fn merges_multifile_manifest() {
        let version = SourceDoc {
            file_name: "pkg.yaml".to_string(),
            root: parse_yaml(
                r#"
PackageIdentifier: Example.App
PackageVersion: 1.0.0
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.9.0
"#,
            ),
            manifest_type: "version".to_string(),
            package_version: "1.0.0".to_string(),
            warnings: Vec::new(),
        };
        let installer = SourceDoc {
            file_name: "pkg.installer.yaml".to_string(),
            root: parse_yaml(
                r#"
PackageIdentifier: Example.App
PackageVersion: 1.0.0
InstallerType: exe
Installers:
  - Architecture: x64
    InstallerUrl: https://example.invalid/a.exe
    InstallerSha256: AAAAA
ManifestType: installer
ManifestVersion: 1.9.0
"#,
            ),
            manifest_type: "installer".to_string(),
            package_version: "1.0.0".to_string(),
            warnings: Vec::new(),
        };
        let default_locale = SourceDoc {
            file_name: "pkg.locale.en-US.yaml".to_string(),
            root: parse_yaml(
                r#"
PackageIdentifier: Example.App
PackageVersion: 1.0.0
PackageLocale: en-US
PackageName: Example
Publisher: Example Inc
ManifestType: defaultLocale
ManifestVersion: 1.9.0
"#,
            ),
            manifest_type: "defaultLocale".to_string(),
            package_version: "1.0.0".to_string(),
            warnings: Vec::new(),
        };

        let merged = merge_docs(vec![version, installer, default_locale]).unwrap();
        assert_eq!(
            get_required_string(&merged, "PackageIdentifier").unwrap(),
            "Example.App"
        );
        assert_eq!(
            get_required_string(&merged, "PackageName").unwrap(),
            "Example"
        );
        assert_eq!(
            get_required_string(&merged, "ManifestType").unwrap(),
            "merged"
        );
    }

    #[test]
    fn installer_hash_excludes_command_protocol_and_extension_changes() {
        let left = parse_yaml(
            r#"
PackageIdentifier: Example.App
PackageVersion: 1.0.0
PackageName: Example
Publisher: Example Inc
Commands: [foo]
Protocols: [foo]
FileExtensions: [foo]
InstallerType: exe
Installers:
  - Architecture: x64
    InstallerUrl: https://example.invalid/a.exe
    InstallerSha256: AAAAA
ManifestType: merged
ManifestVersion: 1.9.0
"#,
        );
        let right = parse_yaml(
            r#"
PackageIdentifier: Example.App
PackageVersion: 1.0.0
PackageName: Example
Publisher: Example Inc
Commands: [bar]
Protocols: [bar]
FileExtensions: [bar]
InstallerType: exe
Installers:
  - Architecture: x64
    InstallerUrl: https://example.invalid/a.exe
    InstallerSha256: AAAAA
ManifestType: merged
ManifestVersion: 1.9.0
"#,
        );

        let left_hash = build_installer_records(&left).unwrap();
        let right_hash = build_installer_records(&right).unwrap();

        assert_eq!(left_hash, right_hash);
    }

    #[test]
    fn canonicalization_omits_null_fields() {
        let manifest = parse_yaml(
            r#"
PackageIdentifier: Example.App
PackageVersion: 1.0.0
ManifestType: merged
ManifestVersion: 1.9.0
Dependencies:
PackageUrl: https://example.invalid
"#,
        );

        let canonical = canonicalize_full_manifest(&manifest).unwrap();
        let object = canonical.as_object().unwrap();

        assert!(!object.contains_key("Dependencies"));
        assert_eq!(
            object.get("PackageUrl").and_then(JsonValue::as_str),
            Some("https://example.invalid")
        );
    }

    #[test]
    fn canonicalization_omits_empty_objects_and_arrays() {
        let manifest = parse_yaml(
            r#"
PackageIdentifier: Example.App
PackageVersion: 1.0.0
ManifestType: merged
ManifestVersion: 1.9.0
InstallerSwitches:
  InstallLocation:
Tags: []
PackageUrl: https://example.invalid
"#,
        );

        let canonical = canonicalize_full_manifest(&manifest).unwrap();
        let object = canonical.as_object().unwrap();

        assert!(!object.contains_key("InstallerSwitches"));
        assert!(!object.contains_key("Tags"));
        assert_eq!(
            object.get("PackageUrl").and_then(JsonValue::as_str),
            Some("https://example.invalid")
        );
    }

    #[test]
    fn normalize_publisher_drops_legal_entity_suffixes() {
        assert_eq!(normalize_publisher("Example Inc"), "example");
        assert_eq!(normalize_publisher("Example Holdings LLC"), "example");
    }

    #[test]
    fn compute_version_snapshot_preserves_numeric_package_version_text() {
        let temp = tempdir().unwrap();
        let repo_root = temp.path();
        let version_dir = repo_root
            .join("manifests")
            .join("j")
            .join("Japplis")
            .join("Watch")
            .join("1.10");
        fs::create_dir_all(&version_dir).unwrap();

        fs::write(
            version_dir.join("Japplis.Watch.yaml"),
            r#"
PackageIdentifier: Japplis.Watch
PackageVersion: 1.10
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.10.0
"#,
        )
        .unwrap();
        fs::write(
            version_dir.join("Japplis.Watch.installer.yaml"),
            r#"
PackageIdentifier: Japplis.Watch
PackageVersion: 1.10
InstallerType: exe
Installers:
  - Architecture: x64
    InstallerUrl: https://example.invalid/japplis-watch.exe
    InstallerSha256: AAAAA
ManifestType: installer
ManifestVersion: 1.10.0
"#,
        )
        .unwrap();
        fs::write(
            version_dir.join("Japplis.Watch.locale.en-US.yaml"),
            r#"
PackageIdentifier: Japplis.Watch
PackageVersion: 1.10
PackageLocale: en-US
PackageName: Japplis Watch
Publisher: Japplis
ManifestType: defaultLocale
ManifestVersion: 1.10.0
"#,
        )
        .unwrap();

        let snapshot =
            compute_version_snapshot(repo_root, &version_dir, "manifests/j/Japplis/Watch/1.10")
                .unwrap();
        let published_manifest = String::from_utf8(snapshot.published_manifest_bytes).unwrap();

        assert_eq!(snapshot.package_version, "1.10");
        assert!(published_manifest.contains("1.10"));
    }

    #[test]
    fn compute_version_snapshot_preserves_trailing_zero_package_version_text() {
        let temp = tempdir().unwrap();
        let repo_root = temp.path();
        let version_dir = repo_root
            .join("manifests")
            .join("e")
            .join("Example")
            .join("App")
            .join("3.0");
        fs::create_dir_all(&version_dir).unwrap();

        fs::write(
            version_dir.join("Example.App.yaml"),
            r#"
PackageIdentifier: Example.App
PackageVersion: 3.0
ManifestType: singleton
ManifestVersion: 1.10.0
PackageLocale: en-US
PackageName: Example App
Publisher: Example
InstallerType: exe
Installers:
  - Architecture: x64
    InstallerUrl: https://example.invalid/app.exe
    InstallerSha256: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
"#,
        )
        .unwrap();

        let snapshot =
            compute_version_snapshot(repo_root, &version_dir, "manifests/e/Example/App/3.0")
                .unwrap();
        let published_manifest = String::from_utf8(snapshot.published_manifest_bytes).unwrap();

        assert_eq!(snapshot.package_version, "3.0");
        assert_eq!(
            extract_top_level_scalar_string(&published_manifest, "PackageVersion").as_deref(),
            Some("3.0")
        );
    }

    #[test]
    fn computes_snapshot_from_winget_cli_multifile_fixture() {
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("winget-cli")
            .join("src")
            .join("AppInstallerCLITests")
            .join("TestData")
            .join("MultiFileManifestV1_12");
        if !fixture_dir.is_dir() {
            return;
        }

        let temp = tempdir().unwrap();
        let repo_root = temp.path();
        let version_dir = repo_root
            .join("manifests")
            .join("m")
            .join("microsoft")
            .join("msixsdk")
            .join("1.7.32");
        fs::create_dir_all(&version_dir).unwrap();

        for entry in fs::read_dir(&fixture_dir).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), version_dir.join(entry.file_name())).unwrap();
        }

        let snapshot = compute_version_snapshot(
            repo_root,
            &version_dir,
            "manifests/m/microsoft/msixsdk/1.7.32",
        )
        .unwrap();
        let published_manifest = String::from_utf8(snapshot.published_manifest_bytes).unwrap();

        assert_eq!(snapshot.package_id, "microsoft.msixsdk");
        assert_eq!(snapshot.package_version, "1.7.32");
        assert_eq!(snapshot.source_file_count, 4);
        assert!(published_manifest.contains("PackageName: MSIX SDK"));
        assert!(published_manifest.contains("PackageIdentifier: microsoft.msixsdk"));
    }

    #[test]
    fn strips_display_version_from_manifest_bytes() {
        let snapshot = ComputedVersionSnapshot {
            version_dir: "manifests/n/NHNCorporation/Dooray!Messenger/2.2.2.1".to_string(),
            package_id: "NHNCorporation.Dooray!Messenger".to_string(),
            package_version: "2.2.2.1".to_string(),
            channel: String::new(),
            index_projection: VersionIndexProjection::default(),
            version_content_sha256: Vec::new(),
            installers: Vec::new(),
            version_installer_sha256: Vec::new(),
            published_manifest_sha256: Vec::new(),
            published_manifest_relpath: String::new(),
            published_manifest_bytes: br#"
PackageIdentifier: NHNCorporation.Dooray!Messenger
PackageVersion: 2.2.2.1
ManifestType: merged
ManifestVersion: 1.10.0
AppsAndFeaturesEntries:
  - ProductCode: DoorayMessenger
    DisplayVersion: 2.2.2
Installers:
  - Architecture: x64
    AppsAndFeaturesEntries:
      - ProductCode: DoorayMessenger
        DisplayVersion: 2.2.2
"#
            .to_vec(),
            source_file_count: 3,
        };

        let temp = tempdir().unwrap();
        let stripped =
            retain_display_versions_in_snapshot(temp.path(), &snapshot, &BTreeSet::new()).unwrap();
        let display_versions =
            extract_display_versions_from_manifest_bytes(&stripped.published_manifest_bytes)
                .unwrap();

        assert!(display_versions.is_empty());
    }
}
