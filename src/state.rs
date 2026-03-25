use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone)]
pub struct StoredFile {
    pub path: String,
    pub version_dir: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub raw_sha256: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct StoredVersion {
    pub version_dir: String,
    pub package_id: String,
    pub package_version: String,
    pub channel: String,
    pub published_manifest_relpath: String,
    pub published_manifest_sha256: Vec<u8>,
    pub version_content_sha256: Vec<u8>,
    pub version_installer_sha256: Vec<u8>,
    pub source_file_count: usize,
}

#[derive(Debug, Clone)]
pub struct StoredPackage {
    pub package_id: String,
    pub version_count: usize,
    pub version_data_relpath: String,
    pub package_publish_sha256: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PublishedFile {
    pub relpath: String,
    pub kind: String,
    pub owner_package_id: Option<String>,
    pub sha256: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct BuildVersionChange {
    pub version_dir: String,
    pub package_id: String,
    pub change_kind: String,
    pub content_changed: bool,
    pub installer_changed: bool,
    pub old_content_sha256: Option<Vec<u8>>,
    pub new_content_sha256: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct BuildPackageChange {
    pub package_id: String,
    pub change_kind: String,
    pub publish_changed: bool,
    pub installer_revalidate: bool,
    pub old_publish_sha256: Option<Vec<u8>>,
    pub new_publish_sha256: Option<Vec<u8>>,
}

pub struct CurrentStateUpdate<'a> {
    pub finished_unix: i64,
    pub last_successful_unix: i64,
    pub files: &'a [StoredFile],
    pub versions: &'a [StoredVersion],
    pub packages: &'a [StoredPackage],
    pub published_files: &'a [PublishedFile],
}

pub struct StateStore {
    root: PathBuf,
    conn: Connection,
}

impl StateStore {
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

    pub fn staging_root(&self) -> PathBuf {
        self.root.join("staging")
    }

    pub fn validation_queue_path(&self) -> PathBuf {
        self.root.join("validation-queue.json")
    }

    pub fn mutable_db_path(&self) -> PathBuf {
        self.root.join("writer").join("mutable-v2.db")
    }

    pub fn begin_build(&self, started_unix: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO builds(started_at_unix, status) VALUES(?1, 'running')",
            params![started_unix],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn mark_build_failed(&self, build_id: i64, finished_unix: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE builds SET finished_at_unix = ?2, status = 'failed', error_text = ?3 WHERE build_id = ?1",
            params![build_id, finished_unix, error],
        )?;
        Ok(())
    }

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

    pub fn load_versions_current(&self) -> Result<HashMap<String, StoredVersion>> {
        let mut statement = self.conn.prepare(
            "SELECT version_dir, package_id, package_version, channel, published_manifest_relpath, published_manifest_sha256, version_content_sha256, version_installer_sha256, source_file_count FROM versions_current",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(StoredVersion {
                version_dir: row.get(0)?,
                package_id: row.get(1)?,
                package_version: row.get(2)?,
                channel: row.get(3)?,
                published_manifest_relpath: row.get(4)?,
                published_manifest_sha256: row.get(5)?,
                version_content_sha256: row.get(6)?,
                version_installer_sha256: row.get(7)?,
                source_file_count: row.get::<_, i64>(8)? as usize,
            })
        })?;

        let mut result = HashMap::new();
        for row in rows {
            let version = row?;
            result.insert(version.version_dir.clone(), version);
        }
        Ok(result)
    }

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

    pub fn replace_current_state(
        &mut self,
        build_id: i64,
        update: CurrentStateUpdate<'_>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;

        tx.execute("DELETE FROM files_current", [])?;
        tx.execute("DELETE FROM version_files_current", [])?;
        tx.execute("DELETE FROM versions_current", [])?;
        tx.execute("DELETE FROM packages_current", [])?;
        tx.execute("DELETE FROM published_files_current", [])?;

        {
            let mut statement = tx.prepare(
                "INSERT INTO files_current(path, version_dir, size, mtime_ns, raw_sha256) VALUES(?1, ?2, ?3, ?4, ?5)",
            )?;
            for file in update.files {
                statement.execute(params![
                    file.path,
                    file.version_dir,
                    file.size as i64,
                    file.mtime_ns,
                    file.raw_sha256,
                ])?;
            }
        }

        {
            let mut statement = tx.prepare(
                "INSERT INTO version_files_current(version_dir, path, raw_sha256) VALUES(?1, ?2, ?3)",
            )?;
            for file in update.files {
                statement.execute(params![file.version_dir, file.path, file.raw_sha256])?;
            }
        }

        {
            let mut statement = tx.prepare(
                "INSERT INTO versions_current(version_dir, package_id, package_version, channel, published_manifest_relpath, published_manifest_sha256, version_content_sha256, version_installer_sha256, source_file_count)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for version in update.versions {
                statement.execute(params![
                    version.version_dir,
                    version.package_id,
                    version.package_version,
                    version.channel,
                    version.published_manifest_relpath,
                    version.published_manifest_sha256,
                    version.version_content_sha256,
                    version.version_installer_sha256,
                    version.source_file_count as i64,
                ])?;
            }
        }

        {
            let mut statement = tx.prepare(
                "INSERT INTO packages_current(package_id, version_count, version_data_relpath, package_publish_sha256)
                 VALUES(?1, ?2, ?3, ?4)",
            )?;
            for package in update.packages {
                statement.execute(params![
                    package.package_id,
                    package.version_count as i64,
                    package.version_data_relpath,
                    package.package_publish_sha256,
                ])?;
            }
        }

        {
            let mut statement = tx.prepare(
                "INSERT INTO published_files_current(relpath, kind, owner_package_id, sha256) VALUES(?1, ?2, ?3, ?4)",
            )?;
            for file in update.published_files {
                statement.execute(params![
                    file.relpath,
                    file.kind,
                    file.owner_package_id,
                    file.sha256,
                ])?;
            }
        }

        tx.execute(
            "INSERT INTO meta(key, value) VALUES('last_successful_unix_epoch', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![update.last_successful_unix.to_string()],
        )?;
        tx.execute(
            "UPDATE builds SET finished_at_unix = ?2, status = 'published', error_text = NULL WHERE build_id = ?1",
            params![build_id, update.finished_unix],
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
                version_installer_sha256 BLOB NOT NULL,
                status TEXT NOT NULL,
                validated_at_unix INTEGER NOT NULL,
                PRIMARY KEY(package_id, package_version, channel, version_installer_sha256)
            );
            ",
        )?;
        Ok(())
    }
}
