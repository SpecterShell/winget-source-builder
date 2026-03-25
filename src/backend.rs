use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::CatalogFormat;
use crate::adapter::package_published_index;
use crate::manifest::{VersionIndexProjection, normalize_rel, sha256_bytes};
#[cfg(not(windows))]
use crate::mszip;
use crate::state::{StoredPackage, StoredVersion};
use crate::version::compare_version_and_channel;

const PUBLISHER_METADATA_KIND: i64 = 5;

pub(crate) fn run_rust_backend(
    workspace_root: &Path,
    stage_root: &Path,
    final_versions: &HashMap<String, StoredVersion>,
    previous_packages: &HashMap<String, StoredPackage>,
    touched_packages: &BTreeSet<String>,
    last_successful_unix: i64,
    format: CatalogFormat,
) -> Result<()> {
    match format {
        CatalogFormat::V1 => run_rust_backend_v1(workspace_root, stage_root, final_versions),
        CatalogFormat::V2 => run_rust_backend_v2(
            workspace_root,
            stage_root,
            final_versions,
            previous_packages,
            touched_packages,
            last_successful_unix,
        ),
    }
}

fn run_rust_backend_v1(
    workspace_root: &Path,
    stage_root: &Path,
    final_versions: &HashMap<String, StoredVersion>,
) -> Result<()> {
    let versions_by_package = build_version_records(final_versions)?;
    let publish_db_path = stage_root.join("index-publish.db");
    write_index_db_v1(
        &publish_db_path,
        &versions_by_package,
        current_unix_epoch()?,
    )?;
    package_published_index(
        workspace_root,
        stage_root,
        &publish_db_path,
        CatalogFormat::V1,
    )
}

fn run_rust_backend_v2(
    workspace_root: &Path,
    stage_root: &Path,
    final_versions: &HashMap<String, StoredVersion>,
    previous_packages: &HashMap<String, StoredPackage>,
    touched_packages: &BTreeSet<String>,
    last_successful_unix: i64,
) -> Result<()> {
    let package_builds =
        build_package_records_v2(final_versions, previous_packages, touched_packages)?;
    write_changed_package_files(stage_root, &package_builds, touched_packages)?;

    let publish_db_path = stage_root.join("index-publish.db");
    write_index_db_v2(
        &publish_db_path,
        &package_builds,
        last_successful_unix,
        current_unix_epoch()?,
    )?;
    package_published_index(
        workspace_root,
        stage_root,
        &publish_db_path,
        CatalogFormat::V2,
    )
}

#[derive(Debug, Clone)]
struct VersionRecord {
    stored: StoredVersion,
    projection: VersionIndexProjection,
}

#[derive(Debug, Clone)]
struct PackageBuildRecordV2 {
    package_id: String,
    version_data_relpath: String,
    package_publish_sha256: Vec<u8>,
    version_data_bytes: Option<Vec<u8>>,
    latest_name: String,
    latest_version: String,
    latest_moniker: Option<String>,
    latest_arp_min_version: Option<String>,
    latest_arp_max_version: Option<String>,
    tags: Vec<String>,
    commands: Vec<String>,
    package_family_names: Vec<String>,
    product_codes: Vec<String>,
    upgrade_codes: Vec<String>,
    normalized_names: Vec<String>,
    normalized_publishers: Vec<String>,
}

fn build_version_records(
    final_versions: &HashMap<String, StoredVersion>,
) -> Result<BTreeMap<String, Vec<VersionRecord>>> {
    let mut result = BTreeMap::<String, Vec<VersionRecord>>::new();
    for version in final_versions.values() {
        let projection = parse_index_projection(version)?;
        result
            .entry(version.package_id.clone())
            .or_default()
            .push(VersionRecord {
                stored: version.clone(),
                projection,
            });
    }

    for versions in result.values_mut() {
        versions.sort_by(compare_version_records);
    }

    Ok(result)
}

fn build_package_records_v2(
    final_versions: &HashMap<String, StoredVersion>,
    previous_packages: &HashMap<String, StoredPackage>,
    touched_packages: &BTreeSet<String>,
) -> Result<BTreeMap<String, PackageBuildRecordV2>> {
    let versions_by_package = build_version_records(final_versions)?;
    let mut result = BTreeMap::new();

    for (package_id, versions) in versions_by_package {
        let latest = versions
            .first()
            .ok_or_else(|| anyhow!("package {package_id} has no versions"))?;

        let artifact = if touched_packages.contains(&package_id) {
            let version_data_bytes = build_version_data_bytes(&versions)?;
            let package_publish_sha256 = sha256_bytes(&version_data_bytes);
            let version_data_relpath =
                build_version_data_relpath(&package_id, &package_publish_sha256);
            (
                version_data_relpath,
                package_publish_sha256,
                Some(version_data_bytes),
            )
        } else {
            let previous = previous_packages.get(&package_id).ok_or_else(|| {
                anyhow!("missing previous package artifact for unchanged package {package_id}")
            })?;
            (
                previous.version_data_relpath.clone(),
                previous.package_publish_sha256.clone(),
                None,
            )
        };

        let mut tags = BTreeSet::new();
        let mut commands = BTreeSet::new();
        let mut package_family_names = BTreeSet::new();
        let mut product_codes = BTreeSet::new();
        let mut upgrade_codes = BTreeSet::new();
        let mut normalized_names = BTreeSet::new();
        let mut normalized_publishers = BTreeSet::new();

        for version in &versions {
            tags.extend(version.projection.tags.iter().cloned());
            commands.extend(version.projection.commands.iter().cloned());
            package_family_names.extend(version.projection.package_family_names.iter().cloned());
            product_codes.extend(version.projection.product_codes.iter().cloned());
            upgrade_codes.extend(version.projection.upgrade_codes.iter().cloned());
            normalized_names.extend(version.projection.normalized_names.iter().cloned());
            normalized_publishers.extend(version.projection.normalized_publishers.iter().cloned());
        }

        result.insert(
            package_id.clone(),
            PackageBuildRecordV2 {
                package_id,
                version_data_relpath: artifact.0,
                package_publish_sha256: artifact.1,
                version_data_bytes: artifact.2,
                latest_name: if latest.projection.package_name.is_empty() {
                    latest.stored.package_id.clone()
                } else {
                    latest.projection.package_name.clone()
                },
                latest_version: latest.stored.package_version.clone(),
                latest_moniker: latest.projection.moniker.clone(),
                latest_arp_min_version: latest.projection.arp_min_version.clone(),
                latest_arp_max_version: latest.projection.arp_max_version.clone(),
                tags: tags.into_iter().collect(),
                commands: commands.into_iter().collect(),
                package_family_names: package_family_names.into_iter().collect(),
                product_codes: product_codes.into_iter().collect(),
                upgrade_codes: upgrade_codes.into_iter().collect(),
                normalized_names: normalized_names.into_iter().collect(),
                normalized_publishers: normalized_publishers.into_iter().collect(),
            },
        );
    }

    Ok(result)
}

fn parse_index_projection(version: &StoredVersion) -> Result<VersionIndexProjection> {
    let Some(json) = version.index_projection_json.as_deref() else {
        bail!(
            "version {} is missing index projection data; rerun with backend rust after state backfill",
            version.version_dir
        );
    };

    serde_json::from_str(json).with_context(|| {
        format!(
            "failed to deserialize index projection for {}",
            version.version_dir
        )
    })
}

fn write_changed_package_files(
    stage_root: &Path,
    packages: &BTreeMap<String, PackageBuildRecordV2>,
    touched_packages: &BTreeSet<String>,
) -> Result<()> {
    for package_id in touched_packages {
        let Some(package) = packages.get(package_id) else {
            continue;
        };
        let Some(bytes) = &package.version_data_bytes else {
            continue;
        };

        let abs_path = stage_root.join(&package.version_data_relpath);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&abs_path, bytes)
            .with_context(|| format!("failed to write {}", abs_path.display()))?;
    }

    Ok(())
}

fn write_index_db_v2(
    db_path: &Path,
    packages: &BTreeMap<String, PackageBuildRecordV2>,
    update_tracking_base: i64,
    last_write_time: i64,
) -> Result<()> {
    let conn = recreate_sqlite_db(db_path)?;
    conn.execute_batch(
        "
        CREATE TABLE metadata(
            [name] TEXT PRIMARY KEY NOT NULL,
            [value] TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE packages(
            rowid INTEGER PRIMARY KEY,
            [id] TEXT NOT NULL,
            [name] TEXT NOT NULL,
            [moniker] TEXT,
            [latest_version] TEXT NOT NULL,
            [arp_min_version] TEXT,
            [arp_max_version] TEXT,
            [hash] BLOB
        );
        CREATE TABLE tags2(
            rowid INTEGER PRIMARY KEY,
            [tag] TEXT NOT NULL
        );
        CREATE TABLE tags2_map(
            [tag] INT64 NOT NULL,
            [package] INT64 NOT NULL,
            PRIMARY KEY([tag], [package])
        ) WITHOUT ROWID;
        CREATE TABLE commands2(
            rowid INTEGER PRIMARY KEY,
            [command] TEXT NOT NULL
        );
        CREATE TABLE commands2_map(
            [command] INT64 NOT NULL,
            [package] INT64 NOT NULL,
            PRIMARY KEY([command], [package])
        ) WITHOUT ROWID;
        CREATE TABLE pfns2(
            [pfn] TEXT NOT NULL,
            [package] INT64 NOT NULL,
            PRIMARY KEY([pfn], [package])
        ) WITHOUT ROWID;
        CREATE TABLE productcodes2(
            [productcode] TEXT NOT NULL,
            [package] INT64 NOT NULL,
            PRIMARY KEY([productcode], [package])
        ) WITHOUT ROWID;
        CREATE TABLE upgradecodes2(
            [upgradecode] TEXT NOT NULL,
            [package] INT64 NOT NULL,
            PRIMARY KEY([upgradecode], [package])
        ) WITHOUT ROWID;
        CREATE TABLE norm_names2(
            [norm_name] TEXT NOT NULL,
            [package] INT64 NOT NULL,
            PRIMARY KEY([norm_name], [package])
        ) WITHOUT ROWID;
        CREATE TABLE norm_publishers2(
            [norm_publisher] TEXT NOT NULL,
            [package] INT64 NOT NULL,
            PRIMARY KEY([norm_publisher], [package])
        ) WITHOUT ROWID;
        ",
    )?;

    let tx = conn.unchecked_transaction()?;
    insert_metadata(
        &tx,
        2,
        0,
        last_write_time,
        Some(update_tracking_base.max(0)),
    )?;

    let mut package_rowids = BTreeMap::new();
    {
        let mut statement = tx.prepare(
            "INSERT INTO packages(id, name, moniker, latest_version, arp_min_version, arp_max_version, hash)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        for package in packages.values() {
            statement.execute(params![
                package.package_id,
                package.latest_name,
                package.latest_moniker,
                package.latest_version,
                package.latest_arp_min_version,
                package.latest_arp_max_version,
                package.package_publish_sha256,
            ])?;
            package_rowids.insert(package.package_id.clone(), tx.last_insert_rowid());
        }
    }

    insert_simple_map_table(
        &tx,
        SimpleMapTableSpec {
            value_table: "tags2",
            value_column: "tag",
            map_table: "tags2_map",
            map_value_column: "tag",
        },
        packages,
        &package_rowids,
        |package| package.tags.iter(),
    )?;
    insert_simple_map_table(
        &tx,
        SimpleMapTableSpec {
            value_table: "commands2",
            value_column: "command",
            map_table: "commands2_map",
            map_value_column: "command",
        },
        packages,
        &package_rowids,
        |package| package.commands.iter(),
    )?;
    insert_direct_table(&tx, "pfns2", "pfn", packages, &package_rowids, |package| {
        package.package_family_names.iter()
    })?;
    insert_direct_table(
        &tx,
        "productcodes2",
        "productcode",
        packages,
        &package_rowids,
        |package| package.product_codes.iter(),
    )?;
    insert_direct_table(
        &tx,
        "upgradecodes2",
        "upgradecode",
        packages,
        &package_rowids,
        |package| package.upgrade_codes.iter(),
    )?;
    insert_direct_table(
        &tx,
        "norm_names2",
        "norm_name",
        packages,
        &package_rowids,
        |package| package.normalized_names.iter(),
    )?;
    insert_direct_table(
        &tx,
        "norm_publishers2",
        "norm_publisher",
        packages,
        &package_rowids,
        |package| package.normalized_publishers.iter(),
    )?;

    tx.commit()?;
    conn.execute_batch("VACUUM")?;
    Ok(())
}

fn write_index_db_v1(
    db_path: &Path,
    versions_by_package: &BTreeMap<String, Vec<VersionRecord>>,
    last_write_time: i64,
) -> Result<()> {
    let conn = recreate_sqlite_db(db_path)?;
    conn.execute_batch(
        "
        CREATE TABLE metadata(
            [name] TEXT PRIMARY KEY NOT NULL,
            [value] TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE ids(
            rowid INTEGER PRIMARY KEY,
            [id] TEXT NOT NULL
        );
        CREATE TABLE names(
            rowid INTEGER PRIMARY KEY,
            [name] TEXT NOT NULL
        );
        CREATE TABLE monikers(
            rowid INTEGER PRIMARY KEY,
            [moniker] TEXT NOT NULL
        );
        CREATE TABLE versions(
            rowid INTEGER PRIMARY KEY,
            [version] TEXT NOT NULL
        );
        CREATE TABLE channels(
            rowid INTEGER PRIMARY KEY,
            [channel] TEXT NOT NULL
        );
        CREATE TABLE pathparts(
            rowid INTEGER PRIMARY KEY,
            [parent] INT64,
            [pathpart] TEXT NOT NULL
        );
        CREATE TABLE manifest(
            rowid INTEGER PRIMARY KEY,
            [id] INT64 NOT NULL,
            [name] INT64,
            [moniker] INT64,
            [version] INT64 NOT NULL,
            [channel] INT64 NOT NULL,
            [pathpart] INT64 NOT NULL
        );
        CREATE TABLE tags(
            rowid INTEGER PRIMARY KEY,
            [tag] TEXT NOT NULL
        );
        CREATE TABLE tags_map(
            [manifest] INT64 NOT NULL,
            [tag] INT64 NOT NULL,
            PRIMARY KEY([tag], [manifest])
        );
        CREATE TABLE commands(
            rowid INTEGER PRIMARY KEY,
            [command] TEXT NOT NULL
        );
        CREATE TABLE commands_map(
            [manifest] INT64 NOT NULL,
            [command] INT64 NOT NULL,
            PRIMARY KEY([command], [manifest])
        );
        CREATE TABLE pfns(
            rowid INTEGER PRIMARY KEY,
            [pfn] TEXT NOT NULL
        );
        CREATE TABLE pfns_map(
            [manifest] INT64 NOT NULL,
            [pfn] INT64 NOT NULL,
            PRIMARY KEY([pfn], [manifest])
        );
        CREATE TABLE productcodes(
            rowid INTEGER PRIMARY KEY,
            [productcode] TEXT NOT NULL
        );
        CREATE TABLE productcodes_map(
            [manifest] INT64 NOT NULL,
            [productcode] INT64 NOT NULL,
            PRIMARY KEY([productcode], [manifest])
        );
        CREATE TABLE norm_names(
            rowid INTEGER PRIMARY KEY,
            [norm_name] TEXT NOT NULL
        );
        CREATE TABLE norm_names_map(
            [manifest] INT64 NOT NULL,
            [norm_name] INT64 NOT NULL,
            PRIMARY KEY([norm_name], [manifest])
        );
        CREATE TABLE norm_publishers(
            rowid INTEGER PRIMARY KEY,
            [norm_publisher] TEXT NOT NULL
        );
        CREATE TABLE norm_publishers_map(
            [manifest] INT64 NOT NULL,
            [norm_publisher] INT64 NOT NULL,
            PRIMARY KEY([norm_publisher], [manifest])
        );
        CREATE TABLE manifest_metadata(
            [manifest] INT64 NOT NULL,
            [metadata] INT64 NOT NULL,
            [value] TEXT,
            PRIMARY KEY([manifest], [metadata])
        );
        ",
    )?;

    let tx = conn.unchecked_transaction()?;
    insert_metadata(&tx, 1, 2, last_write_time, None)?;

    let mut ids = ValueCache::new("ids", "id", &tx)?;
    let mut names = ValueCache::new("names", "name", &tx)?;
    let mut monikers = ValueCache::new("monikers", "moniker", &tx)?;
    let mut versions = ValueCache::new("versions", "version", &tx)?;
    let mut channels = ValueCache::new("channels", "channel", &tx)?;
    let mut tags = ValueCache::new("tags", "tag", &tx)?;
    let mut commands = ValueCache::new("commands", "command", &tx)?;
    let mut pfns = ValueCache::new("pfns", "pfn", &tx)?;
    let mut product_codes = ValueCache::new("productcodes", "productcode", &tx)?;
    let mut norm_names = ValueCache::new("norm_names", "norm_name", &tx)?;
    let mut norm_publishers = ValueCache::new("norm_publishers", "norm_publisher", &tx)?;
    let mut pathparts = PathPartCache::new(&tx)?;

    let mut manifest_statement = tx.prepare(
        "INSERT INTO manifest(id, name, moniker, version, channel, pathpart)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut metadata_statement =
        tx.prepare("INSERT INTO manifest_metadata(manifest, metadata, value) VALUES(?1, ?2, ?3)")?;
    let mut tag_map_statement = tx.prepare("INSERT INTO tags_map(manifest, tag) VALUES(?1, ?2)")?;
    let mut command_map_statement =
        tx.prepare("INSERT INTO commands_map(manifest, command) VALUES(?1, ?2)")?;
    let mut pfn_map_statement = tx.prepare("INSERT INTO pfns_map(manifest, pfn) VALUES(?1, ?2)")?;
    let mut product_code_map_statement =
        tx.prepare("INSERT INTO productcodes_map(manifest, productcode) VALUES(?1, ?2)")?;
    let mut norm_name_map_statement =
        tx.prepare("INSERT INTO norm_names_map(manifest, norm_name) VALUES(?1, ?2)")?;
    let mut norm_publisher_map_statement =
        tx.prepare("INSERT INTO norm_publishers_map(manifest, norm_publisher) VALUES(?1, ?2)")?;

    for versions_for_package in versions_by_package.values() {
        for version in versions_for_package {
            let id_id = ids.ensure(&version.stored.package_id)?;
            let name_value = if version.projection.package_name.is_empty() {
                version.stored.package_id.as_str()
            } else {
                version.projection.package_name.as_str()
            };
            let name_id = names.ensure(name_value)?;
            let moniker_id = version
                .projection
                .moniker
                .as_deref()
                .map(|value| monikers.ensure(value))
                .transpose()?;
            let version_id = versions.ensure(&version.stored.package_version)?;
            let channel_id = channels.ensure(&version.stored.channel)?;
            let pathpart_id = pathparts.ensure(&version.stored.published_manifest_relpath)?;

            manifest_statement.execute(params![
                id_id,
                name_id,
                moniker_id,
                version_id,
                channel_id,
                pathpart_id,
            ])?;
            let manifest_id = tx.last_insert_rowid();

            if let Some(publisher) = version.projection.publisher.as_deref() {
                metadata_statement.execute(params![
                    manifest_id,
                    PUBLISHER_METADATA_KIND,
                    publisher
                ])?;
            }

            insert_map_values(
                manifest_id,
                version.projection.tags.iter(),
                &mut tags,
                &mut tag_map_statement,
            )?;
            insert_map_values(
                manifest_id,
                version.projection.commands.iter(),
                &mut commands,
                &mut command_map_statement,
            )?;
            insert_map_values(
                manifest_id,
                version.projection.package_family_names.iter(),
                &mut pfns,
                &mut pfn_map_statement,
            )?;
            insert_map_values(
                manifest_id,
                version.projection.product_codes.iter(),
                &mut product_codes,
                &mut product_code_map_statement,
            )?;
            insert_map_values(
                manifest_id,
                version.projection.normalized_names.iter(),
                &mut norm_names,
                &mut norm_name_map_statement,
            )?;
            insert_map_values(
                manifest_id,
                version.projection.normalized_publishers.iter(),
                &mut norm_publishers,
                &mut norm_publisher_map_statement,
            )?;
        }
    }

    drop(norm_publisher_map_statement);
    drop(norm_name_map_statement);
    drop(product_code_map_statement);
    drop(pfn_map_statement);
    drop(command_map_statement);
    drop(tag_map_statement);
    drop(metadata_statement);
    drop(manifest_statement);
    drop(pathparts);
    drop(norm_publishers);
    drop(norm_names);
    drop(product_codes);
    drop(pfns);
    drop(commands);
    drop(tags);
    drop(channels);
    drop(versions);
    drop(monikers);
    drop(names);
    drop(ids);
    tx.commit()?;
    conn.execute_batch("VACUUM")?;
    Ok(())
}

fn recreate_sqlite_db(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if db_path.exists() {
        fs::remove_file(db_path)
            .with_context(|| format!("failed to remove {}", db_path.display()))?;
    }

    Connection::open(db_path).with_context(|| format!("failed to open {}", db_path.display()))
}

fn insert_metadata(
    tx: &rusqlite::Transaction<'_>,
    major_version: u32,
    minor_version: u32,
    last_write_time: i64,
    update_tracking_base: Option<i64>,
) -> Result<()> {
    let mut statement = tx.prepare("INSERT INTO metadata(name, value) VALUES(?1, ?2)")?;
    let database_identifier = format!(
        "{{{}}}",
        Uuid::new_v4().hyphenated().to_string().to_uppercase()
    );
    statement.execute(params!["databaseIdentifier", database_identifier])?;
    statement.execute(params!["majorVersion", major_version.to_string()])?;
    statement.execute(params!["minorVersion", minor_version.to_string()])?;
    statement.execute(params!["lastwritetime", last_write_time.to_string()])?;

    if let Some(update_tracking_base) = update_tracking_base {
        statement.execute(params![
            "updateTrackingBase",
            update_tracking_base.to_string()
        ])?;
    }

    Ok(())
}

struct ValueCache<'a> {
    values: BTreeMap<String, i64>,
    insert_statement: rusqlite::Statement<'a>,
}

impl<'a> ValueCache<'a> {
    fn new(table: &str, column: &str, tx: &'a rusqlite::Transaction<'a>) -> Result<Self> {
        Ok(Self {
            values: BTreeMap::new(),
            insert_statement: tx.prepare(&format!("INSERT INTO {table}({column}) VALUES(?1)"))?,
        })
    }

    fn ensure(&mut self, value: &str) -> Result<i64> {
        if let Some(existing) = self.values.get(value) {
            return Ok(*existing);
        }

        let rowid = self.insert_statement.insert(params![value])?;
        self.values.insert(value.to_string(), rowid);
        Ok(rowid)
    }
}

struct PathPartCache<'a> {
    parts: BTreeMap<(Option<i64>, String), i64>,
    insert_statement: rusqlite::Statement<'a>,
}

impl<'a> PathPartCache<'a> {
    fn new(tx: &'a rusqlite::Transaction<'a>) -> Result<Self> {
        Ok(Self {
            parts: BTreeMap::new(),
            insert_statement: tx
                .prepare("INSERT INTO pathparts(parent, pathpart) VALUES(?1, ?2)")?,
        })
    }

    fn ensure(&mut self, relative_path: &str) -> Result<i64> {
        let mut parent = None;
        for segment in relative_path.split('/') {
            let key = (parent, segment.to_string());
            let current = if let Some(existing) = self.parts.get(&key) {
                *existing
            } else {
                let rowid = self.insert_statement.insert(params![parent, segment])?;
                self.parts.insert(key, rowid);
                rowid
            };
            parent = Some(current);
        }

        parent.ok_or_else(|| anyhow!("relative path {relative_path} has no segments"))
    }
}

fn insert_map_values<'a, I>(
    manifest_id: i64,
    values: I,
    cache: &mut ValueCache<'a>,
    map_statement: &mut rusqlite::Statement<'a>,
) -> Result<()>
where
    I: Iterator<Item = &'a String>,
{
    for value in values {
        let value_id = cache.ensure(value)?;
        map_statement.execute(params![manifest_id, value_id])?;
    }
    Ok(())
}

struct SimpleMapTableSpec<'a> {
    value_table: &'a str,
    value_column: &'a str,
    map_table: &'a str,
    map_value_column: &'a str,
}

fn insert_simple_map_table<'a, F, I>(
    tx: &rusqlite::Transaction<'_>,
    spec: SimpleMapTableSpec<'_>,
    packages: &'a BTreeMap<String, PackageBuildRecordV2>,
    package_rowids: &BTreeMap<String, i64>,
    values: F,
) -> Result<()>
where
    F: Fn(&'a PackageBuildRecordV2) -> I,
    I: Iterator<Item = &'a String>,
{
    let mut distinct = BTreeMap::<String, i64>::new();
    {
        let mut statement = tx.prepare(&format!(
            "INSERT INTO {}({}) VALUES(?1)",
            spec.value_table, spec.value_column
        ))?;
        for package in packages.values() {
            for value in values(package) {
                if distinct.contains_key(value) {
                    continue;
                }
                let rowid = statement.insert(params![value])?;
                distinct.insert(value.clone(), rowid);
            }
        }
    }

    let mut map_statement = tx.prepare(&format!(
        "INSERT INTO {}({}, package) VALUES(?1, ?2)",
        spec.map_table, spec.map_value_column
    ))?;
    for package in packages.values() {
        let package_rowid = *package_rowids
            .get(&package.package_id)
            .ok_or_else(|| anyhow!("missing rowid for package {}", package.package_id))?;
        for value in values(package) {
            let value_rowid = *distinct
                .get(value)
                .ok_or_else(|| anyhow!("missing rowid for {} value {value}", spec.value_table))?;
            map_statement.execute(params![value_rowid, package_rowid])?;
        }
    }

    Ok(())
}

fn insert_direct_table<'a, F, I>(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    value_column: &str,
    packages: &'a BTreeMap<String, PackageBuildRecordV2>,
    package_rowids: &BTreeMap<String, i64>,
    values: F,
) -> Result<()>
where
    F: Fn(&'a PackageBuildRecordV2) -> I,
    I: Iterator<Item = &'a String>,
{
    let mut statement = tx.prepare(&format!(
        "INSERT INTO {table}({value_column}, package) VALUES(?1, ?2)"
    ))?;
    for package in packages.values() {
        let package_rowid = *package_rowids
            .get(&package.package_id)
            .ok_or_else(|| anyhow!("missing rowid for package {}", package.package_id))?;
        for value in values(package) {
            statement.execute(params![value, package_rowid])?;
        }
    }
    Ok(())
}

fn build_version_data_bytes(versions: &[VersionRecord]) -> Result<Vec<u8>> {
    let mut ordered = versions.to_vec();
    ordered.sort_by(compare_version_records);

    let mut yaml = String::from("sV: 1.0\nvD:\n");
    for version in &ordered {
        yaml.push_str("- v: ");
        yaml.push_str(&escape_yaml_scalar(&version.stored.package_version));
        yaml.push('\n');

        if let Some(arp_min_version) = &version.projection.arp_min_version {
            yaml.push_str("  aMiV: ");
            yaml.push_str(&escape_yaml_scalar(arp_min_version));
            yaml.push('\n');
        }

        if let Some(arp_max_version) = &version.projection.arp_max_version {
            yaml.push_str("  aMaV: ");
            yaml.push_str(&escape_yaml_scalar(arp_max_version));
            yaml.push('\n');
        }

        yaml.push_str("  rP: ");
        yaml.push_str(&escape_yaml_scalar(
            &version.stored.published_manifest_relpath,
        ));
        yaml.push('\n');
        yaml.push_str("  s256H: ");
        yaml.push_str(&hex::encode(&version.stored.published_manifest_sha256));
        yaml.push('\n');
    }

    compress_mszip(yaml.as_bytes())
}

fn build_version_data_relpath(package_id: &str, package_publish_sha256: &[u8]) -> String {
    let hash_prefix = &hex::encode(package_publish_sha256)[0..8];
    normalize_rel(
        &std::path::PathBuf::from("packages")
            .join(package_id)
            .join(hash_prefix)
            .join("versionData.mszyml")
            .to_string_lossy(),
    )
}

fn escape_yaml_scalar(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }

    let requires_quotes = value.starts_with([' ', '-', ':', '[', '{', '!', '&', '*', '?', '@'])
        || value.ends_with([' ', ':'])
        || value.contains(['\n', '\r', '"', '#'])
        || value.parse::<f64>().is_ok()
        || value.eq_ignore_ascii_case("null")
        || value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("false");

    if requires_quotes {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

fn compare_version_records(left: &VersionRecord, right: &VersionRecord) -> std::cmp::Ordering {
    compare_version_and_channel(
        &left.stored.package_version,
        &left.stored.channel,
        &right.stored.package_version,
        &right.stored.channel,
    )
    .then_with(|| left.stored.version_dir.cmp(&right.stored.version_dir))
}

#[cfg(windows)]
fn compress_mszip(input: &[u8]) -> Result<Vec<u8>> {
    use std::ptr;

    use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, GetLastError};
    use windows_sys::Win32::Storage::Compression::{
        COMPRESS_ALGORITHM_MSZIP, COMPRESSOR_HANDLE, CloseCompressor, Compress, CreateCompressor,
    };

    let mut compressor: COMPRESSOR_HANDLE = ptr::null_mut();
    let created =
        unsafe { CreateCompressor(COMPRESS_ALGORITHM_MSZIP, ptr::null(), &mut compressor) };
    if created == 0 {
        bail!("CreateCompressor failed with error {}", unsafe {
            GetLastError()
        });
    }

    let result = (|| {
        let mut required_size = 0usize;
        let first = unsafe {
            Compress(
                compressor,
                input.as_ptr() as _,
                input.len(),
                ptr::null_mut(),
                0,
                &mut required_size,
            )
        };
        if first == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_INSUFFICIENT_BUFFER {
                bail!("Compress size query failed with error {error}");
            }
        }

        let mut output = vec![0u8; required_size];
        let mut written = 0usize;
        let compressed = unsafe {
            Compress(
                compressor,
                input.as_ptr() as _,
                input.len(),
                output.as_mut_ptr() as _,
                output.len(),
                &mut written,
            )
        };
        if compressed == 0 {
            bail!("Compress failed with error {}", unsafe { GetLastError() });
        }
        output.truncate(written);
        Ok(output)
    })();

    unsafe {
        CloseCompressor(compressor);
    }

    result
}

#[cfg(not(windows))]
fn compress_mszip(input: &[u8]) -> Result<Vec<u8>> {
    mszip::compress_all(input)
}

fn current_unix_epoch() -> Result<i64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system time predates unix epoch")?
        .as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_version_data_relpath_with_lowercase_hash_prefix() {
        let relpath = build_version_data_relpath("Example.App", &[0xAB, 0xCD, 0xEF, 0x01]);
        assert_eq!(relpath, "packages/Example.App/abcdef01/versionData.mszyml");
    }

    #[test]
    fn compare_package_versions_prefers_newer_textual_versions() {
        assert!(crate::version::compare_versions("3.16.2", "3.15.2").is_gt());
        assert!(crate::version::compare_versions("3.10", "3.2").is_gt());
        assert!(crate::version::compare_versions("1.0.0-preview.2", "1.0.0-preview.1").is_gt());
    }
}
