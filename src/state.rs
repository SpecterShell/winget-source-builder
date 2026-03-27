use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::{BackendKind, CatalogFormat};

#[derive(Debug, Clone, Serialize)]
pub struct StoredFile {
    pub path: String,
    pub version_dir: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub raw_sha256: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredVersion {
    pub version_dir: String,
    pub package_id: String,
    pub package_version: String,
    pub channel: String,
    pub index_projection_json: Option<String>,
    pub installers_json: Option<String>,
    pub published_manifest_relpath: String,
    pub published_manifest_sha256: Vec<u8>,
    pub version_content_sha256: Vec<u8>,
    pub version_installer_sha256: Vec<u8>,
    pub source_file_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredPackage {
    pub package_id: String,
    pub version_count: usize,
    pub version_data_relpath: String,
    pub package_publish_sha256: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishedFile {
    pub relpath: String,
    pub kind: String,
    pub owner_package_id: Option<String>,
    pub sha256: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildVersionChange {
    pub version_dir: String,
    pub package_id: String,
    pub change_kind: String,
    pub content_changed: bool,
    pub installer_changed: bool,
    pub old_content_sha256: Option<Vec<u8>>,
    pub new_content_sha256: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildPackageChange {
    pub package_id: String,
    pub change_kind: String,
    pub publish_changed: bool,
    pub installer_revalidate: bool,
    pub old_publish_sha256: Option<Vec<u8>>,
    pub new_publish_sha256: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildRecord {
    pub build_id: i64,
    pub started_at_unix: i64,
    pub finished_at_unix: Option<i64>,
    pub status: String,
    pub error_text: Option<String>,
}

pub struct WorkingStateUpdate<'a> {
    pub build_id: i64,
    pub finished_unix: i64,
    pub files: &'a [StoredFile],
    pub versions: &'a [StoredVersion],
    pub packages: &'a [StoredPackage],
    pub index_version: CatalogFormat,
    pub backend: BackendKind,
    pub build_status: &'a str,
}

pub struct StateStore {
    root: PathBuf,
    conn: Connection,
}

impl StateStore {
    /// Opens the state store under `root`, creating the directory and schema if needed.
    ///
    /// # Arguments
    ///
    /// * `root` - State directory that will contain `state.sqlite`, staging, and cache files.
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)
            .with_context(|| format!("failed to create state root {}", root.display()))?;
        let db_path = root.join("state.sqlite");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("failed to open state db {}", db_path.display()))?;
        let store = Self {
            root: root.to_path_buf(),
            conn,
        };
        store.init()?;
        Ok(store)
    }

    /// Returns the directory that holds per-build staged outputs.
    pub fn staging_root(&self) -> PathBuf {
        self.root.join("staging")
    }

    /// Returns the staging directory for a specific build id.
    ///
    /// # Arguments
    ///
    /// * `build_id` - Build identifier previously allocated by [`StateStore::begin_build`].
    pub fn stage_root_for_build(&self, build_id: i64) -> PathBuf {
        self.staging_root().join(format!("build-{build_id}"))
    }

    /// Returns the path of the persisted validation queue file.
    pub fn validation_queue_path(&self) -> PathBuf {
        self.root.join("validation-queue.json")
    }

    /// Returns the mutable writer database path for the selected index version.
    ///
    /// # Arguments
    ///
    /// * `format` - Index family whose writer cache is being addressed.
    pub fn mutable_db_path_for_format(&self, format: CatalogFormat) -> PathBuf {
        let file_name = match format {
            CatalogFormat::V1 => "mutable-v1.db",
            CatalogFormat::V2 => "mutable-v2.db",
        };
        self.root.join("writer").join(file_name)
    }

    /// Returns the latest build id whose staged artifacts are considered current.
    pub fn last_staged_build_id(&self) -> Result<Option<i64>> {
        self.meta_i64("last_staged_build_id")
    }

    /// Returns the index version recorded for the latest staged build, if any.
    pub fn last_staged_index_version(&self) -> Result<Option<CatalogFormat>> {
        Ok(self
            .meta_string("last_staged_index_version")?
            .and_then(|value| match value.as_str() {
                "v1" => Some(CatalogFormat::V1),
                "v2" => Some(CatalogFormat::V2),
                _ => None,
            }))
    }

    /// Returns the backend recorded for the latest staged build, if any.
    pub fn last_staged_backend(&self) -> Result<Option<BackendKind>> {
        Ok(self
            .meta_string("last_staged_backend")?
            .and_then(|value| match value.as_str() {
                "wingetutil" => Some(BackendKind::Wingetutil),
                "rust" => Some(BackendKind::Rust),
                _ => None,
            }))
    }

    /// Returns the latest successfully published build id, if any.
    pub fn last_published_build_id(&self) -> Result<Option<i64>> {
        self.meta_i64("last_published_build_id")
    }

    /// Inserts a new running build record and returns its build id.
    ///
    /// # Arguments
    ///
    /// * `started_unix` - Build start time as a Unix epoch in seconds.
    pub fn begin_build(&self, started_unix: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO builds(started_at_unix, status) VALUES(?1, 'running')",
            params![started_unix],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Marks a build as failed and persists the captured error text.
    ///
    /// # Arguments
    ///
    /// * `build_id` - Build record to update.
    /// * `finished_unix` - Failure timestamp as a Unix epoch in seconds.
    /// * `error` - Rendered error text to persist for later inspection.
    pub fn mark_build_failed(&self, build_id: i64, finished_unix: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE builds SET finished_at_unix = ?2, status = 'failed', error_text = ?3 WHERE build_id = ?1",
            params![build_id, finished_unix, error],
        )?;
        Ok(())
    }

    /// Returns the last successful publish timestamp, or `0` when nothing has been published yet.
    pub fn last_successful_build_epoch(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'last_successful_unix_epoch'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0))
    }

    /// Loads the current file snapshot keyed by normalized relative path.
    pub fn load_files_current(&self) -> Result<HashMap<String, StoredFile>> {
        let mut statement = self
            .conn
            .prepare("SELECT path, version_dir, size, mtime_ns, raw_sha256 FROM files_current")?;
        let rows = statement.query_map([], |row| {
            Ok(StoredFile {
                path: row.get(0)?,
                version_dir: row.get(1)?,
                size: row.get::<_, i64>(2)? as u64,
                mtime_ns: row.get(3)?,
                raw_sha256: row.get(4)?,
            })
        })?;

        let mut result = HashMap::new();
        for row in rows {
            let file = row?;
            result.insert(file.path.clone(), file);
        }
        Ok(result)
    }

    /// Loads the current version snapshot keyed by normalized version directory.
    pub fn load_versions_current(&self) -> Result<HashMap<String, StoredVersion>> {
        let mut statement = self.conn.prepare(
            "SELECT version_dir, package_id, package_version, channel, index_projection_json, installers_json, published_manifest_relpath, published_manifest_sha256, version_content_sha256, version_installer_sha256, source_file_count FROM versions_current",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(StoredVersion {
                version_dir: row.get(0)?,
                package_id: row.get(1)?,
                package_version: row.get(2)?,
                channel: row.get(3)?,
                index_projection_json: row.get(4)?,
                installers_json: row.get(5)?,
                published_manifest_relpath: row.get(6)?,
                published_manifest_sha256: row.get(7)?,
                version_content_sha256: row.get(8)?,
                version_installer_sha256: row.get(9)?,
                source_file_count: row.get::<_, i64>(10)? as usize,
            })
        })?;

        let mut result = HashMap::new();
        for row in rows {
            let version = row?;
            result.insert(version.version_dir.clone(), version);
        }
        Ok(result)
    }

    /// Loads the current package snapshot keyed by package identifier.
    pub fn load_packages_current(&self) -> Result<HashMap<String, StoredPackage>> {
        let mut statement = self.conn.prepare(
            "SELECT package_id, version_count, version_data_relpath, package_publish_sha256 FROM packages_current",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(StoredPackage {
                package_id: row.get(0)?,
                version_count: row.get::<_, i64>(1)? as usize,
                version_data_relpath: row.get(2)?,
                package_publish_sha256: row.get(3)?,
            })
        })?;

        let mut result = HashMap::new();
        for row in rows {
            let package = row?;
            result.insert(package.package_id.clone(), package);
        }
        Ok(result)
    }

    /// Loads the tracked published files keyed by relative output path.
    pub fn load_published_files_current(&self) -> Result<HashMap<String, PublishedFile>> {
        let mut statement = self.conn.prepare(
            "SELECT relpath, kind, owner_package_id, sha256 FROM published_files_current",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(PublishedFile {
                relpath: row.get(0)?,
                kind: row.get(1)?,
                owner_package_id: row.get(2)?,
                sha256: row.get(3)?,
            })
        })?;

        let mut result = HashMap::new();
        for row in rows {
            let file = row?;
            result.insert(file.relpath.clone(), file);
        }
        Ok(result)
    }

    /// Loads build records in descending build-id order, optionally capped to `limit`.
    ///
    /// # Arguments
    ///
    /// * `limit` - Maximum number of build records to return, or `None` for all rows.
    pub fn load_builds(&self, limit: Option<usize>) -> Result<Vec<BuildRecord>> {
        let sql = match limit {
            Some(_) => {
                "SELECT build_id, started_at_unix, finished_at_unix, status, error_text
                 FROM builds
                 ORDER BY build_id DESC
                 LIMIT ?1"
            }
            None => {
                "SELECT build_id, started_at_unix, finished_at_unix, status, error_text
                 FROM builds
                 ORDER BY build_id DESC"
            }
        };

        let mut statement = self.conn.prepare(sql)?;
        let mut rows = if let Some(limit) = limit {
            statement.query(params![limit as i64])?
        } else {
            statement.query([])?
        };

        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            result.push(BuildRecord {
                build_id: row.get(0)?,
                started_at_unix: row.get(1)?,
                finished_at_unix: row.get(2)?,
                status: row.get(3)?,
                error_text: row.get(4)?,
            });
        }
        Ok(result)
    }

    /// Loads a single build record by id.
    ///
    /// # Arguments
    ///
    /// * `build_id` - Build identifier to look up.
    pub fn load_build(&self, build_id: i64) -> Result<Option<BuildRecord>> {
        self.conn
            .query_row(
                "SELECT build_id, started_at_unix, finished_at_unix, status, error_text
                 FROM builds WHERE build_id = ?1",
                params![build_id],
                |row| {
                    Ok(BuildRecord {
                        build_id: row.get(0)?,
                        started_at_unix: row.get(1)?,
                        finished_at_unix: row.get(2)?,
                        status: row.get(3)?,
                        error_text: row.get(4)?,
                    })
                },
            )
            .optional()
            .context("failed to load build record")
    }

    /// Returns counts for files, versions, packages, and published files in current state.
    pub fn current_counts(&self) -> Result<(i64, i64, i64, i64)> {
        Ok((
            self.table_count("files_current")?,
            self.table_count("versions_current")?,
            self.table_count("packages_current")?,
            self.table_count("published_files_current")?,
        ))
    }

    /// Prunes old build records while retaining the newest `keep_last` entries.
    ///
    /// # Arguments
    ///
    /// * `keep_last` - Number of newest build rows that must always be preserved.
    /// * `older_than_before_unix` - Optional cutoff that further restricts which older rows may be deleted.
    pub fn prune_build_records(
        &self,
        keep_last: usize,
        older_than_before_unix: Option<i64>,
    ) -> Result<usize> {
        let keep_last = keep_last as i64;
        let deleted = match older_than_before_unix {
            Some(cutoff) => self.conn.execute(
                "DELETE FROM builds
                 WHERE build_id NOT IN (
                     SELECT build_id FROM builds ORDER BY build_id DESC LIMIT ?1
                 )
                 AND COALESCE(finished_at_unix, started_at_unix) < ?2",
                params![keep_last, cutoff],
            )?,
            None => self.conn.execute(
                "DELETE FROM builds
                 WHERE build_id NOT IN (
                     SELECT build_id FROM builds ORDER BY build_id DESC LIMIT ?1
                 )",
                params![keep_last],
            )?,
        };
        Ok(deleted)
    }

    /// Clears published-output tracking without touching working state snapshots.
    pub fn clear_published_tracking(&self) -> Result<usize> {
        let deleted = self
            .conn
            .execute("DELETE FROM published_files_current", [])?;
        self.conn.execute(
            "DELETE FROM meta WHERE key IN ('last_published_build_id', 'last_successful_unix_epoch')",
            [],
        )?;
        Ok(deleted)
    }

    /// Replaces the recorded version-level diff for a build.
    ///
    /// # Arguments
    ///
    /// * `build_id` - Build whose version diff rows should be replaced.
    /// * `changes` - Fully computed version-level diff for the build.
    pub fn record_version_changes(
        &self,
        build_id: i64,
        changes: &[BuildVersionChange],
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM build_version_changes WHERE build_id = ?1",
            params![build_id],
        )?;

        let mut statement = self.conn.prepare(
            "INSERT INTO build_version_changes(build_id, version_dir, package_id, change_kind, content_changed, installer_changed, old_content_sha256, new_content_sha256)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;

        for change in changes {
            statement.execute(params![
                build_id,
                change.version_dir,
                change.package_id,
                change.change_kind,
                change.content_changed as i64,
                change.installer_changed as i64,
                change.old_content_sha256,
                change.new_content_sha256,
            ])?;
        }

        Ok(())
    }

    /// Replaces the recorded package-level diff for a build.
    ///
    /// # Arguments
    ///
    /// * `build_id` - Build whose package diff rows should be replaced.
    /// * `changes` - Fully computed package-level diff for the build.
    pub fn record_package_changes(
        &self,
        build_id: i64,
        changes: &[BuildPackageChange],
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM build_package_changes WHERE build_id = ?1",
            params![build_id],
        )?;

        let mut statement = self.conn.prepare(
            "INSERT INTO build_package_changes(build_id, package_id, change_kind, publish_changed, installer_revalidate, old_publish_sha256, new_publish_sha256)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        for change in changes {
            statement.execute(params![
                build_id,
                change.package_id,
                change.change_kind,
                change.publish_changed as i64,
                change.installer_revalidate as i64,
                change.old_publish_sha256,
                change.new_publish_sha256,
            ])?;
        }

        Ok(())
    }

    /// Atomically replaces the working state snapshot after a successful staged build.
    ///
    /// # Arguments
    ///
    /// * `update` - Replacement working-state snapshot plus staged-build metadata.
    pub fn replace_working_state(&mut self, update: WorkingStateUpdate<'_>) -> Result<()> {
        let tx = self.conn.transaction()?;

        // Working state is replaced atomically after a successful staged build so later commands
        // never observe a partially refreshed snapshot.
        tx.execute("DELETE FROM files_current", [])?;
        tx.execute("DELETE FROM version_files_current", [])?;
        tx.execute("DELETE FROM versions_current", [])?;
        tx.execute("DELETE FROM packages_current", [])?;

        // Batch insert with pre-allocated capacity for better performance
        Self::batch_insert_files(&tx, update.files)?;
        Self::batch_insert_version_files(&tx, update.files)?;
        Self::batch_insert_versions(&tx, update.versions)?;
        Self::batch_insert_packages(&tx, update.packages)?;

        upsert_meta_tx(&tx, "last_staged_build_id", update.build_id.to_string())?;
        upsert_meta_tx(
            &tx,
            "last_staged_index_version",
            update.index_version.as_str().to_string(),
        )?;
        upsert_meta_tx(
            &tx,
            "last_staged_backend",
            update.backend.as_str().to_string(),
        )?;
        tx.execute(
            "UPDATE builds SET finished_at_unix = ?2, status = ?3, error_text = NULL WHERE build_id = ?1",
            params![update.build_id, update.finished_unix, update.build_status],
        )?;

        tx.commit()?;
        Ok(())
    }

    fn batch_insert_files(tx: &rusqlite::Transaction<'_>, files: &[StoredFile]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let mut statement = tx.prepare(
            "INSERT INTO files_current(path, version_dir, size, mtime_ns, raw_sha256) VALUES(?1, ?2, ?3, ?4, ?5)",
        )?;
        for file in files {
            statement.execute(params![
                file.path,
                file.version_dir,
                file.size as i64,
                file.mtime_ns,
                file.raw_sha256,
            ])?;
        }
        Ok(())
    }

    fn batch_insert_version_files(
        tx: &rusqlite::Transaction<'_>,
        files: &[StoredFile],
    ) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let mut statement = tx.prepare(
            "INSERT INTO version_files_current(version_dir, path, raw_sha256) VALUES(?1, ?2, ?3)",
        )?;
        for file in files {
            statement.execute(params![file.version_dir, file.path, file.raw_sha256])?;
        }
        Ok(())
    }

    fn batch_insert_versions(
        tx: &rusqlite::Transaction<'_>,
        versions: &[StoredVersion],
    ) -> Result<()> {
        if versions.is_empty() {
            return Ok(());
        }
        let mut statement = tx.prepare(
            "INSERT INTO versions_current(version_dir, package_id, package_version, channel, index_projection_json, installers_json, published_manifest_relpath, published_manifest_sha256, version_content_sha256, version_installer_sha256, source_file_count)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for version in versions {
            statement.execute(params![
                version.version_dir,
                version.package_id,
                version.package_version,
                version.channel,
                version.index_projection_json,
                version.installers_json,
                version.published_manifest_relpath,
                version.published_manifest_sha256,
                version.version_content_sha256,
                version.version_installer_sha256,
                version.source_file_count as i64,
            ])?;
        }
        Ok(())
    }

    fn batch_insert_packages(
        tx: &rusqlite::Transaction<'_>,
        packages: &[StoredPackage],
    ) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }
        let mut statement = tx.prepare(
            "INSERT INTO packages_current(package_id, version_count, version_data_relpath, package_publish_sha256)
             VALUES(?1, ?2, ?3, ?4)",
        )?;
        for package in packages {
            statement.execute(params![
                package.package_id,
                package.version_count as i64,
                package.version_data_relpath,
                package.package_publish_sha256,
            ])?;
        }
        Ok(())
    }

    /// Atomically replaces published tracking after a successful `publish`.
    ///
    /// # Arguments
    ///
    /// * `build_id` - Build that produced the newly published output tree.
    /// * `finished_unix` - Publish completion time as a Unix epoch in seconds.
    /// * `published_files` - Exact set of files now expected in the published output tree.
    pub fn replace_published_state(
        &mut self,
        build_id: i64,
        finished_unix: i64,
        published_files: &[PublishedFile],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;

        tx.execute("DELETE FROM published_files_current", [])?;
        {
            let mut statement = tx.prepare(
                "INSERT INTO published_files_current(relpath, kind, owner_package_id, sha256) VALUES(?1, ?2, ?3, ?4)",
            )?;
            for file in published_files {
                statement.execute(params![
                    file.relpath,
                    file.kind,
                    file.owner_package_id,
                    file.sha256,
                ])?;
            }
        }

        upsert_meta_tx(&tx, "last_successful_unix_epoch", finished_unix.to_string())?;
        upsert_meta_tx(&tx, "last_published_build_id", build_id.to_string())?;
        tx.execute(
            "UPDATE builds SET finished_at_unix = ?2, status = 'published', error_text = NULL WHERE build_id = ?1",
            params![build_id, finished_unix],
        )?;

        tx.commit()?;
        Ok(())
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS meta(
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS builds(
                build_id INTEGER PRIMARY KEY,
                started_at_unix INTEGER NOT NULL,
                finished_at_unix INTEGER NULL,
                status TEXT NOT NULL,
                error_text TEXT NULL
            );
            CREATE TABLE IF NOT EXISTS files_current(
                path TEXT PRIMARY KEY,
                version_dir TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime_ns INTEGER NOT NULL,
                raw_sha256 BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS version_files_current(
                version_dir TEXT NOT NULL,
                path TEXT NOT NULL,
                raw_sha256 BLOB NOT NULL,
                PRIMARY KEY(version_dir, path)
            );
            CREATE TABLE IF NOT EXISTS versions_current(
                version_dir TEXT PRIMARY KEY,
                package_id TEXT NOT NULL,
                package_version TEXT NOT NULL,
                channel TEXT NOT NULL,
                index_projection_json TEXT NULL,
                installers_json TEXT NULL,
                published_manifest_relpath TEXT NOT NULL,
                published_manifest_sha256 BLOB NOT NULL,
                version_content_sha256 BLOB NOT NULL,
                version_installer_sha256 BLOB NOT NULL,
                source_file_count INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS packages_current(
                package_id TEXT PRIMARY KEY,
                version_count INTEGER NOT NULL,
                version_data_relpath TEXT NOT NULL,
                package_publish_sha256 BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS published_files_current(
                relpath TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                owner_package_id TEXT NULL,
                sha256 BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS build_version_changes(
                build_id INTEGER NOT NULL,
                version_dir TEXT NOT NULL,
                package_id TEXT NOT NULL,
                change_kind TEXT NOT NULL,
                content_changed INTEGER NOT NULL,
                installer_changed INTEGER NOT NULL,
                old_content_sha256 BLOB NULL,
                new_content_sha256 BLOB NULL,
                PRIMARY KEY(build_id, version_dir)
            );
            CREATE TABLE IF NOT EXISTS build_package_changes(
                build_id INTEGER NOT NULL,
                package_id TEXT NOT NULL,
                change_kind TEXT NOT NULL,
                publish_changed INTEGER NOT NULL,
                installer_revalidate INTEGER NOT NULL,
                old_publish_sha256 BLOB NULL,
                new_publish_sha256 BLOB NULL,
                PRIMARY KEY(build_id, package_id)
            );
            CREATE TABLE IF NOT EXISTS validation_state(
                package_id TEXT NOT NULL,
                package_version TEXT NOT NULL,
                channel TEXT NOT NULL,
                installer_sha256 TEXT NOT NULL,
                status TEXT NOT NULL,
                validated_at_unix INTEGER NOT NULL,
                PRIMARY KEY(package_id, package_version, channel, installer_sha256)
            );
            ",
        )?;
        self.ensure_column_exists("versions_current", "index_projection_json", "TEXT NULL")?;
        self.ensure_column_exists("versions_current", "installers_json", "TEXT NULL")?;
        Ok(())
    }

    fn meta_string(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("failed to read state metadata")
    }

    fn meta_i64(&self, key: &str) -> Result<Option<i64>> {
        Ok(self
            .meta_string(key)?
            .and_then(|value| value.parse::<i64>().ok()))
    }

    fn ensure_column_exists(&self, table: &str, column: &str, definition: &str) -> Result<()> {
        let mut statement = self
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .with_context(|| format!("failed to inspect table {table}"))?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        for row in rows {
            if row? == column {
                return Ok(());
            }
        }

        self.conn
            .execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                [],
            )
            .with_context(|| format!("failed to add column {column} to {table}"))?;
        Ok(())
    }

    fn table_count(&self, table: &str) -> Result<i64> {
        self.conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .with_context(|| format!("failed to count rows in {table}"))
    }
}

fn upsert_meta_tx(tx: &rusqlite::Transaction<'_>, key: &str, value: String) -> Result<()> {
    tx.execute(
        "INSERT INTO meta(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

impl CatalogFormat {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

impl BackendKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Wingetutil => "wingetutil",
            Self::Rust => "rust",
        }
    }
}
