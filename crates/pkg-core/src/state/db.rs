//! SQLite local state database management (ADR-006, DEC-006).

use rusqlite::{Connection, params};
use std::path::Path;

use crate::domain::capability::Capability;
use crate::domain::installed::InstalledPackage;
use crate::domain::package::{
    Architecture, ArtifactDigest, LifecycleScript, NormalizedPackage, PackageEntry, PackageFormat,
    PackageName, PackageVersion,
};
use crate::error::{Error, Result};

/// Database connection and state operations.
pub struct StateDatabase {
    conn: Connection,
    remote_metadata_columns: bool,
    remote_versioned_provides_column: bool,
}

impl std::fmt::Debug for StateDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateDatabase").finish()
    }
}

/// Transaction record persisted in the state database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionRecord {
    /// Unique transaction identifier.
    pub id: String,
    /// Transaction operation (e.g. `install`, `remove`).
    pub operation: String,
    /// Current transaction phase.
    pub phase: String,
    /// Package name involved.
    pub package_name: String,
    /// Store ID if assigned.
    pub store_id: Option<String>,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Last update timestamp (ISO 8601).
    pub updated_at: String,
    /// JSON details or diagnostic notes.
    pub details: Option<String>,
}

/// A store object recorded in pkg state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreObjectRecord {
    /// Stable store identity.
    pub store_id: String,
    /// Package name recorded for the object.
    pub package_name: String,
    /// Package version recorded for the object.
    pub version: String,
    /// Target architecture.
    pub architecture: String,
    /// Source package format.
    pub format: String,
    /// Digest of the source artifact.
    pub digest: String,
    /// Absolute store path.
    pub store_path: std::path::PathBuf,
    /// Installation timestamp.
    pub installed_at: String,
}

/// A published profile generation recorded in the state database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationRecord {
    pub profile: String,
    pub generation_id: String,
    pub previous_generation: Option<String>,
    pub manifest_path: std::path::PathBuf,
    pub created_at: String,
    pub active: bool,
}

/// Durable receipt metadata for an install/remove publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionReceiptRecord {
    pub transaction_id: String,
    pub plan_identity: String,
    pub previous_generation: Option<String>,
    pub new_generation: Option<String>,
    pub phase: String,
    pub details: String,
}

/// Ownership evidence for one user-space host integration action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationRecord {
    pub profile: String,
    pub package_name: String,
    pub store_id: String,
    pub kind: String,
    pub source_path: std::path::PathBuf,
    pub target_path: std::path::PathBuf,
    pub source_digest: String,
}

/// Parameters for recording a newly promoted store object.
#[derive(Debug, Clone)]
pub struct NewStoreObject<'a> {
    /// Unique store identifier.
    pub store_id: &'a str,
    /// Package name.
    pub name: &'a PackageName,
    /// Package version.
    pub version: &'a PackageVersion,
    /// Architecture.
    pub architecture: &'a Architecture,
    /// Package format.
    pub format: PackageFormat,
    /// Artifact digest.
    pub digest: &'a ArtifactDigest,
    /// Path to store object directory.
    pub store_path: &'a Path,
    /// List of files contained in the package.
    pub files: &'a [std::path::PathBuf],
}

impl StateDatabase {
    /// Opens or creates the SQLite state database at the specified path and runs migrations.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let mut db = Self {
            conn,
            remote_metadata_columns: false,
            remote_versioned_provides_column: false,
        };
        db.init_schema()?;
        db.remote_metadata_columns = has_column(&db.conn, "remote_packages", "constraints_json")?
            && has_column(&db.conn, "remote_packages", "provides_json")?;
        db.remote_versioned_provides_column =
            has_column(&db.conn, "remote_packages", "versioned_provides_json")?;
        Ok(db)
    }

    /// Opens an existing database without running migrations or writing to it.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let remote_metadata_columns = has_column(&conn, "remote_packages", "constraints_json")?
            && has_column(&conn, "remote_packages", "provides_json")?;
        let remote_versioned_provides_column =
            has_column(&conn, "remote_packages", "versioned_provides_json")?;
        Ok(Self {
            conn,
            remote_metadata_columns,
            remote_versioned_provides_column,
        })
    }

    /// Creates an in-memory state database for side-effect-free planning.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn,
            remote_metadata_columns: true,
            remote_versioned_provides_column: true,
        };
        db.init_schema()?;
        Ok(db)
    }

    fn remote_columns(&self) -> &'static str {
        if self.remote_metadata_columns {
            if self.remote_versioned_provides_column {
                "constraints_json, provides_json, versioned_provides_json"
            } else {
                "constraints_json, provides_json, '[]' AS versioned_provides_json"
            }
        } else {
            "'[]' AS constraints_json, '[]' AS provides_json, '[]' AS versioned_provides_json"
        }
    }

    /// Initializes schema and tables.
    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY
            );

            CREATE TABLE IF NOT EXISTS packages (
                profile TEXT NOT NULL,
                name TEXT NOT NULL,
                active_version TEXT NOT NULL,
                active_store_id TEXT NOT NULL,
                installed_at TEXT NOT NULL,
                PRIMARY KEY (profile, name)
            );

            CREATE TABLE IF NOT EXISTS store_objects (
                store_id TEXT PRIMARY KEY NOT NULL,
                package_name TEXT NOT NULL,
                version TEXT NOT NULL,
                architecture TEXT NOT NULL,
                format TEXT NOT NULL,
                digest TEXT NOT NULL,
                store_path TEXT NOT NULL,
                installed_at TEXT NOT NULL,
                is_reachable INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS activations (
                profile TEXT NOT NULL,
                command TEXT NOT NULL,
                store_id TEXT NOT NULL,
                package_name TEXT NOT NULL,
                target_path TEXT NOT NULL,
                PRIMARY KEY (profile, command),
                FOREIGN KEY (store_id) REFERENCES store_objects(store_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS transactions (
                id TEXT PRIMARY KEY NOT NULL,
                operation TEXT NOT NULL,
                phase TEXT NOT NULL,
                package_name TEXT NOT NULL,
                store_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                details TEXT
            );

            CREATE TABLE IF NOT EXISTS installed_files (
                store_id TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                PRIMARY KEY (store_id, relative_path),
                FOREIGN KEY (store_id) REFERENCES store_objects(store_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS generations (
                profile TEXT NOT NULL,
                generation_id TEXT NOT NULL,
                previous_generation TEXT,
                manifest_path TEXT NOT NULL,
                created_at TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (profile, generation_id)
            );

            CREATE TABLE IF NOT EXISTS generation_objects (
                profile TEXT NOT NULL,
                generation_id TEXT NOT NULL,
                store_id TEXT NOT NULL,
                PRIMARY KEY (profile, generation_id, store_id),
                FOREIGN KEY (profile, generation_id) REFERENCES generations(profile, generation_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS generation_runtimes (
                profile TEXT NOT NULL,
                generation_id TEXT NOT NULL,
                runtime_id TEXT NOT NULL,
                PRIMARY KEY (profile, generation_id, runtime_id),
                FOREIGN KEY (profile, generation_id) REFERENCES generations(profile, generation_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS generation_packages (
                profile TEXT NOT NULL,
                generation_id TEXT NOT NULL,
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                store_id TEXT NOT NULL,
                installed_at TEXT NOT NULL,
                PRIMARY KEY (profile, generation_id, name),
                FOREIGN KEY (profile, generation_id) REFERENCES generations(profile, generation_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS generation_activations (
                profile TEXT NOT NULL,
                generation_id TEXT NOT NULL,
                command TEXT NOT NULL,
                store_id TEXT NOT NULL,
                package_name TEXT NOT NULL,
                target_path TEXT NOT NULL,
                PRIMARY KEY (profile, generation_id, command),
                FOREIGN KEY (profile, generation_id) REFERENCES generations(profile, generation_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS runtime_manifests (
                runtime_id TEXT PRIMARY KEY NOT NULL,
                manifest_path TEXT NOT NULL,
                manifest_digest TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS transaction_receipts (
                transaction_id TEXT PRIMARY KEY NOT NULL,
                plan_identity TEXT NOT NULL,
                previous_generation TEXT,
                new_generation TEXT,
                phase TEXT NOT NULL,
                details TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS integrations (
                profile TEXT NOT NULL,
                package_name TEXT NOT NULL,
                store_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                source_path TEXT NOT NULL,
                target_path TEXT NOT NULL,
                source_digest TEXT NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (profile, target_path),
                FOREIGN KEY (store_id) REFERENCES store_objects(store_id) ON DELETE RESTRICT
            );

            CREATE TABLE IF NOT EXISTS repositories (
                id TEXT PRIMARY KEY,
                format TEXT NOT NULL DEFAULT 'deb',
                url TEXT NOT NULL,
                distribution TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS remote_packages (
                repository_id TEXT NOT NULL,
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                architecture TEXT NOT NULL,
                format TEXT NOT NULL,
                digest TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                url TEXT NOT NULL,
                constraints_json TEXT NOT NULL DEFAULT '[]',
                provides_json TEXT NOT NULL DEFAULT '[]',
                versioned_provides_json TEXT NOT NULL DEFAULT '[]',
                FOREIGN KEY(repository_id) REFERENCES repositories(id)
            );

            CREATE INDEX IF NOT EXISTS idx_remote_packages_name ON remote_packages(name);
            "#,
        )?;

        // Migrate databases created before repository format and normalized
        // capability metadata were persisted.  The read-only open path does
        // not call this method and therefore remains side-effect free.
        if !has_column(&self.conn, "repositories", "format")? {
            self.conn.execute(
                "ALTER TABLE repositories ADD COLUMN format TEXT NOT NULL DEFAULT 'deb'",
                [],
            )?;
        }
        if !has_column(&self.conn, "remote_packages", "constraints_json")? {
            self.conn.execute(
                "ALTER TABLE remote_packages ADD COLUMN constraints_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }
        if !has_column(&self.conn, "remote_packages", "provides_json")? {
            self.conn.execute(
                "ALTER TABLE remote_packages ADD COLUMN provides_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }
        if !has_column(&self.conn, "remote_packages", "versioned_provides_json")? {
            self.conn.execute(
                "ALTER TABLE remote_packages ADD COLUMN versioned_provides_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }

        Ok(())
    }

    /// Records an install transaction starting.
    pub fn record_transaction_start(
        &self,
        id: &str,
        operation: &str,
        phase: &str,
        package_name: &str,
        store_id: Option<&str>,
        details: Option<&str>,
    ) -> Result<()> {
        let now = chrono_now();
        self.conn.execute(
            "INSERT INTO transactions (id, operation, phase, package_name, store_id, created_at, updated_at, details)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, operation, phase, package_name, store_id, now, now, details],
        )?;

        Ok(())
    }

    /// Updates transaction phase and timestamp.
    pub fn update_transaction_phase(&self, id: &str, phase: &str) -> Result<()> {
        let now = chrono_now();
        self.conn.execute(
            "UPDATE transactions SET phase = ?1, updated_at = ?2 WHERE id = ?3",
            params![phase, now, id],
        )?;
        // Keep the receipt phase aligned with the transaction journal whenever
        // a receipt was created before filesystem mutation.  The update is
        // intentionally a no-op for legacy transactions that predate receipts.
        self.conn.execute(
            "UPDATE transaction_receipts SET phase = ?1, updated_at = ?2 WHERE transaction_id = ?3",
            params![phase, now, id],
        )?;
        Ok(())
    }

    /// Atomically replaces the snapshot of a remote repository.
    /// This fully drops the old `remote_packages` for this `repository_id` and inserts the new ones.
    pub fn commit_repository_snapshot(
        &self,
        repository_id: &str,
        format: &str,
        url: &str,
        distribution: &str,
        packages: &[crate::domain::package::RemotePackage],
    ) -> Result<()> {
        self.conn.execute("BEGIN", ())?;

        // Upsert repository info
        let now = chrono_now();
        if let Err(e) = self.conn.execute(
            "INSERT INTO repositories (id, format, url, distribution, updated_at) 
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET 
                format=excluded.format, url=excluded.url, distribution=excluded.distribution, updated_at=excluded.updated_at",
            params![repository_id, format, url, distribution, now],
        ) {
            let _ = self.conn.execute("ROLLBACK", ());
            return Err(Error::Database(e));
        }

        // Delete old packages
        if let Err(e) = self.conn.execute(
            "DELETE FROM remote_packages WHERE repository_id = ?1",
            params![repository_id],
        ) {
            let _ = self.conn.execute("ROLLBACK", ());
            return Err(Error::Database(e));
        }

        // Insert new packages
        let mut stmt = match self.conn.prepare(
            "INSERT INTO remote_packages 
            (repository_id, name, version, architecture, format, digest, size_bytes, url, constraints_json, provides_json, versioned_provides_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        ) {
            Ok(s) => s,
            Err(e) => {
                let _ = self.conn.execute("ROLLBACK", ());
                return Err(Error::Database(e));
            }
        };

        for pkg in packages {
            if let Err(e) = stmt.execute(params![
                repository_id,
                pkg.name,
                pkg.version,
                pkg.architecture,
                pkg.format,
                pkg.digest,
                pkg.size_bytes,
                pkg.url,
                serde_json::to_string(&pkg.constraints)?,
                serde_json::to_string(&pkg.provides)?,
                serde_json::to_string(&pkg.versioned_provides)?,
            ]) {
                drop(stmt);
                let _ = self.conn.execute("ROLLBACK", ());
                return Err(Error::Database(e));
            }
        }
        drop(stmt);

        self.conn.execute("COMMIT", ())?;
        Ok(())
    }

    /// Searches for a remote package by exact name in the active snapshots.
    /// Returns the first match.
    pub fn get_remote_package(
        &self,
        name: &str,
    ) -> Result<Option<crate::domain::package::RemotePackage>> {
        let query = format!(
            "SELECT repository_id, name, version, architecture, format, digest, size_bytes, url, {}
                 FROM remote_packages
                 WHERE name = ?1 LIMIT 1",
            self.remote_columns()
        );
        let mut stmt = self.conn.prepare(&query)?;

        let row_result = stmt.query_row(params![name], |row| {
            Ok(crate::domain::package::RemotePackage {
                repository_id: row.get(0)?,
                name: row.get(1)?,
                version: row.get(2)?,
                architecture: row.get(3)?,
                format: row.get(4)?,
                digest: row.get(5)?,
                size_bytes: row.get(6)?,
                url: row.get(7)?,
                constraints: serde_json::from_str(&row.get::<_, String>(8)?).unwrap_or_default(),
                provides: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                versioned_provides: serde_json::from_str(&row.get::<_, String>(10)?)
                    .unwrap_or_default(),
            })
        });

        match row_result {
            Ok(pkg) => Ok(Some(pkg)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Error::Database(e)),
        }
    }

    /// Finds the repository snapshot entry that authenticates an artifact
    /// digest.  The digest is the join key between a downloaded cache object
    /// and the metadata snapshot that selected it.
    pub fn remote_package_by_digest(
        &self,
        digest: &str,
    ) -> Result<Option<(crate::domain::package::RemotePackage, String)>> {
        let query = format!(
            "SELECT p.repository_id, p.name, p.version, p.architecture, p.format,
                    p.digest, p.size_bytes, p.url, {}, r.updated_at
             FROM remote_packages p
             JOIN repositories r ON r.id = p.repository_id
             WHERE lower(p.digest) = lower(?1)
             ORDER BY p.repository_id ASC, p.name ASC, p.version DESC
             LIMIT 1",
            self.remote_columns()
        );
        let mut stmt = self.conn.prepare(&query)?;
        let row = stmt.query_row(params![digest], |row| {
            Ok((
                crate::domain::package::RemotePackage {
                    repository_id: row.get(0)?,
                    name: row.get(1)?,
                    version: row.get(2)?,
                    architecture: row.get(3)?,
                    format: row.get(4)?,
                    digest: row.get(5)?,
                    size_bytes: row.get(6)?,
                    url: row.get(7)?,
                    constraints: serde_json::from_str(&row.get::<_, String>(8)?)
                        .unwrap_or_default(),
                    provides: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                    versioned_provides: serde_json::from_str(&row.get::<_, String>(10)?)
                        .unwrap_or_default(),
                },
                row.get(11)?,
            ))
        });
        match row {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(Error::Database(error)),
        }
    }

    /// Searches for remote packages matching a target specification.
    ///
    /// Supported target formats:
    /// - `repository_id/name` (e.g. `fedora-41/curl`, `arch-extra/curl`)
    /// - `name:format` (e.g. `curl:rpm`, `curl:alpm`, `curl:deb`)
    /// - `name@version` (e.g. `curl@8.9.1-1.fc41`)
    /// - `name` (exact package name match across all repositories)
    pub fn find_remote_candidates(
        &self,
        spec: &str,
    ) -> Result<Vec<crate::domain::package::RemotePackage>> {
        let columns = self.remote_columns();
        let (query_sql, params_vec): (String, Vec<Box<dyn rusqlite::ToSql>>) = if let Some((
            repo,
            name,
        )) =
            spec.split_once('/')
        {
            (
                format!("SELECT repository_id, name, version, architecture, format, digest, size_bytes, url, {columns}
                     FROM remote_packages
                     WHERE name = ?1 AND repository_id = ?2
                     ORDER BY version DESC"),
                vec![Box::new(name.to_string()), Box::new(repo.to_string())],
            )
        } else if let Some((name, format)) = spec.split_once(':') {
            (
                format!("SELECT repository_id, name, version, architecture, format, digest, size_bytes, url, {columns}
                     FROM remote_packages
                     WHERE name = ?1 AND format = ?2
                     ORDER BY version DESC"),
                vec![Box::new(name.to_string()), Box::new(format.to_string())],
            )
        } else if let Some((name, version)) = spec.split_once('@') {
            (
                format!("SELECT repository_id, name, version, architecture, format, digest, size_bytes, url, {columns}
                     FROM remote_packages
                     WHERE name = ?1 AND version = ?2
                     ORDER BY version DESC"),
                vec![Box::new(name.to_string()), Box::new(version.to_string())],
            )
        } else {
            (
                format!("SELECT repository_id, name, version, architecture, format, digest, size_bytes, url, {columns}
                     FROM remote_packages
                     WHERE name = ?1
                     ORDER BY version DESC"),
                vec![Box::new(spec.to_string())],
            )
        };

        let mut stmt = self.conn.prepare(&query_sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(crate::domain::package::RemotePackage {
                repository_id: row.get(0)?,
                name: row.get(1)?,
                version: row.get(2)?,
                architecture: row.get(3)?,
                format: row.get(4)?,
                digest: row.get(5)?,
                size_bytes: row.get(6)?,
                url: row.get(7)?,
                constraints: serde_json::from_str(&row.get::<_, String>(8)?).unwrap_or_default(),
                provides: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                versioned_provides: serde_json::from_str(&row.get::<_, String>(10)?)
                    .unwrap_or_default(),
            })
        })?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    /// Searches for remote packages by name (substring matching) with optional format or repo filters.
    pub fn search_remote_packages_filtered(
        &self,
        query: &str,
        format_filter: Option<&str>,
        repo_filter: Option<&str>,
    ) -> Result<Vec<crate::domain::package::RemotePackage>> {
        let mut sql = format!(
            "SELECT repository_id, name, version, architecture, format, digest, size_bytes, url, {}
                       FROM remote_packages
                       WHERE name LIKE ?1",
            self.remote_columns()
        );

        let pattern = format!("%{}%", query);
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(pattern)];

        if let Some(fmt) = format_filter {
            sql.push_str(&format!(" AND format = ?{}", params.len() + 1));
            params.push(Box::new(fmt.to_string()));
        }

        if let Some(repo) = repo_filter {
            sql.push_str(&format!(" AND repository_id = ?{}", params.len() + 1));
            params.push(Box::new(repo.to_string()));
        }

        sql.push_str(" ORDER BY name ASC, version DESC");

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(crate::domain::package::RemotePackage {
                repository_id: row.get(0)?,
                name: row.get(1)?,
                version: row.get(2)?,
                architecture: row.get(3)?,
                format: row.get(4)?,
                digest: row.get(5)?,
                size_bytes: row.get(6)?,
                url: row.get(7)?,
                constraints: serde_json::from_str(&row.get::<_, String>(8)?).unwrap_or_default(),
                provides: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                versioned_provides: serde_json::from_str(&row.get::<_, String>(10)?)
                    .unwrap_or_default(),
            })
        })?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    /// Searches for remote packages by name (substring matching) across active snapshots.
    pub fn search_remote_packages(
        &self,
        query: &str,
    ) -> Result<Vec<crate::domain::package::RemotePackage>> {
        self.search_remote_packages_filtered(query, None, None)
    }

    /// Lists all transactions that did not complete normally.
    pub fn list_incomplete_transactions(&self) -> Result<Vec<TransactionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, operation, phase, package_name, store_id, created_at, updated_at, details
             FROM transactions
             WHERE phase NOT IN ('Completed', 'FailedClean')",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(TransactionRecord {
                id: row.get(0)?,
                operation: row.get(1)?,
                phase: row.get(2)?,
                package_name: row.get(3)?,
                store_id: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                details: row.get(7)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Checks if a command name is already activated in the specified profile by a DIFFERENT package.
    pub fn find_conflicting_activation(
        &self,
        profile: &str,
        command: &str,
        new_package: &str,
    ) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT package_name FROM activations WHERE profile = ?1 AND command = ?2")?;
        let mut rows = stmt.query(params![profile, command])?;
        if let Some(row) = rows.next()? {
            let existing_package: String = row.get(0)?;
            if existing_package != new_package {
                return Ok(Some(existing_package));
            }
        }
        Ok(None)
    }

    /// Records a newly promoted store object.
    pub fn record_store_object(&self, obj: &NewStoreObject<'_>) -> Result<()> {
        let now = chrono_now();
        self.conn.execute(
            "INSERT INTO store_objects (store_id, package_name, version, architecture, format, digest, store_path, installed_at, is_reachable)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)
             ON CONFLICT(store_id) DO NOTHING",
            params![
                obj.store_id,
                obj.name.as_str(),
                obj.version.as_str(),
                obj.architecture.as_str(),
                obj.format.to_string(),
                obj.digest.to_string(),
                obj.store_path.to_str().unwrap_or(""),
                now
            ],
        )?;

        // Record file ownerships
        for f in obj.files {
            self.conn.execute(
                "INSERT OR IGNORE INTO installed_files (store_id, relative_path) VALUES (?1, ?2)",
                params![obj.store_id, f.to_str().unwrap_or("")],
            )?;
        }

        Ok(())
    }

    /// Records activation of a binary command in a profile.
    pub fn record_activation(
        &self,
        profile: &str,
        command: &str,
        store_id: &str,
        package_name: &str,
        target_path: &Path,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO activations (profile, command, store_id, package_name, target_path)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                profile,
                command,
                store_id,
                package_name,
                target_path.to_str().unwrap_or("")
            ],
        )?;
        Ok(())
    }

    /// Records logical package registration in the profile.
    pub fn record_package(
        &self,
        profile: &str,
        name: &PackageName,
        version: &PackageVersion,
        store_id: &str,
    ) -> Result<()> {
        let now = chrono_now();
        self.conn.execute(
            "INSERT OR REPLACE INTO packages (profile, name, active_version, active_store_id, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![profile, name.as_str(), version.as_str(), store_id, now],
        )?;
        Ok(())
    }

    /// Atomically records the promoted store object, active binaries, and package row.
    /// Keeping these writes in one SQLite transaction preserves the previous package
    /// state if a database constraint or disk failure interrupts an upgrade.
    pub fn commit_install_state(
        &self,
        profile: &str,
        package: &crate::domain::package::NormalizedPackage,
        store_id: &str,
        store_path: &Path,
        files: &[std::path::PathBuf],
        activations: &[(&str, &Path)],
    ) -> Result<()> {
        self.conn.execute("BEGIN IMMEDIATE", [])?;
        let result = (|| -> Result<()> {
            let now = chrono_now();
            self.conn.execute(
                "INSERT INTO store_objects (store_id, package_name, version, architecture, format, digest, store_path, installed_at, is_reachable)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)
                 ON CONFLICT(store_id) DO NOTHING",
                params![
                    store_id,
                    package.name.as_str(),
                    package.version.as_str(),
                    package.architecture.as_str(),
                    package.format.to_string(),
                    package.digest.to_string(),
                    store_path.to_str().unwrap_or(""),
                    now,
                ],
            )?;
            for file in files {
                self.conn.execute(
                    "INSERT OR IGNORE INTO installed_files (store_id, relative_path) VALUES (?1, ?2)",
                    params![store_id, file.to_str().unwrap_or("")],
                )?;
            }
            self.conn.execute(
                "DELETE FROM activations WHERE profile = ?1 AND package_name = ?2",
                params![profile, package.name.as_str()],
            )?;
            for (command, target) in activations {
                self.conn.execute(
                    "INSERT INTO activations (profile, command, store_id, package_name, target_path) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![profile, command, store_id, package.name.as_str(), target.to_str().unwrap_or("")],
                )?;
            }
            self.conn.execute(
                "INSERT INTO packages (profile, name, active_version, active_store_id, installed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(profile, name) DO UPDATE SET active_version=excluded.active_version, active_store_id=excluded.active_store_id, installed_at=excluded.installed_at",
                params![profile, package.name.as_str(), package.version.as_str(), store_id, chrono_now()],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute("COMMIT", [])?;
                Ok(())
            }
            Err(error) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(error)
            }
        }
    }

    /// Records a generation and its retained store-object roots in one SQLite
    /// transaction.  The filesystem pointer is switched by the caller before
    /// this method; recovery compares both records and pointer on startup.
    pub fn record_generation(
        &self,
        generation: &crate::domain::contracts::ActivationGeneration,
        manifest_path: &Path,
        store_ids: &[&str],
    ) -> Result<()> {
        self.conn.execute("BEGIN IMMEDIATE", [])?;
        let result = (|| -> Result<()> {
            let now = chrono_now();
            self.conn.execute(
                "UPDATE generations SET active = 0 WHERE profile = ?1",
                params![generation.profile],
            )?;
            self.conn.execute(
                "INSERT INTO generations (profile, generation_id, previous_generation, manifest_path, created_at, active)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1)
                 ON CONFLICT(profile, generation_id) DO UPDATE SET active=1, manifest_path=excluded.manifest_path",
                params![
                    generation.profile,
                    generation.generation_id,
                    generation.previous_generation,
                    manifest_path.to_string_lossy(),
                    now,
                ],
            )?;
            self.conn.execute(
                "DELETE FROM generation_objects WHERE profile = ?1 AND generation_id = ?2",
                params![generation.profile, generation.generation_id],
            )?;
            for store_id in store_ids {
                self.conn.execute(
                    "INSERT OR IGNORE INTO generation_objects (profile, generation_id, store_id) VALUES (?1, ?2, ?3)",
                    params![generation.profile, generation.generation_id, store_id],
                )?;
            }
            self.conn.execute(
                "DELETE FROM generation_runtimes WHERE profile = ?1 AND generation_id = ?2",
                params![generation.profile, generation.generation_id],
            )?;
            for runtime_id in &generation.runtimes {
                self.conn.execute(
                    "INSERT OR IGNORE INTO generation_runtimes (profile, generation_id, runtime_id) VALUES (?1, ?2, ?3)",
                    params![generation.profile, generation.generation_id, runtime_id],
                )?;
            }
            self.conn.execute(
                "DELETE FROM generation_packages WHERE profile = ?1 AND generation_id = ?2",
                params![generation.profile, generation.generation_id],
            )?;
            self.conn.execute(
                "DELETE FROM generation_activations WHERE profile = ?1 AND generation_id = ?2",
                params![generation.profile, generation.generation_id],
            )?;
            let mut packages = self.conn.prepare(
                "SELECT name, active_version, active_store_id, installed_at FROM packages WHERE profile = ?1",
            )?;
            let package_rows = packages.query_map(params![generation.profile], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            let package_rows = package_rows.collect::<std::result::Result<Vec<_>, _>>()?;
            drop(packages);
            for (name, version, store_id, installed_at) in package_rows {
                self.conn.execute(
                    "INSERT INTO generation_packages (profile, generation_id, name, version, store_id, installed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![generation.profile, generation.generation_id, name, version, store_id, installed_at],
                )?;
            }
            let mut activations = self.conn.prepare(
                "SELECT command, store_id, package_name, target_path FROM activations WHERE profile = ?1",
            )?;
            let activation_rows = activations.query_map(params![generation.profile], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            let activation_rows = activation_rows.collect::<std::result::Result<Vec<_>, _>>()?;
            drop(activations);
            for (command, store_id, package_name, target_path) in activation_rows {
                self.conn.execute(
                    "INSERT INTO generation_activations (profile, generation_id, command, store_id, package_name, target_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![generation.profile, generation.generation_id, command, store_id, package_name, target_path],
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute("COMMIT", [])?;
                Ok(())
            }
            Err(error) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(error)
            }
        }
    }

    /// Persists a transaction receipt, updating it idempotently during
    /// recovery or completion.
    pub fn record_transaction_receipt(
        &self,
        receipt: &crate::domain::contracts::TransactionReceipt,
        details: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO transaction_receipts (transaction_id, plan_identity, previous_generation, new_generation, phase, details, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(transaction_id) DO UPDATE SET plan_identity=excluded.plan_identity,
                previous_generation=excluded.previous_generation, new_generation=excluded.new_generation,
                phase=excluded.phase, details=excluded.details, updated_at=excluded.updated_at",
            params![
                receipt.transaction_id,
                receipt.plan_identity,
                receipt.previous_generation,
                receipt.new_generation,
                receipt.phase,
                details,
                chrono_now(),
            ],
        )?;
        Ok(())
    }

    /// Records the immutable runtime manifest location and content identity.
    pub fn record_runtime_manifest(
        &self,
        runtime_id: &str,
        manifest_path: &Path,
        manifest_digest: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO runtime_manifests (runtime_id, manifest_path, manifest_digest, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(runtime_id) DO UPDATE SET manifest_path=excluded.manifest_path,
                manifest_digest=excluded.manifest_digest",
            params![
                runtime_id,
                manifest_path.to_string_lossy(),
                manifest_digest,
                chrono_now()
            ],
        )?;
        Ok(())
    }

    /// Returns the persisted runtime manifest path and its recorded content
    /// digest.  Launchers use this pair to reject tampered runtime records.
    pub fn runtime_manifest_record(
        &self,
        runtime_id: &str,
    ) -> Result<Option<(std::path::PathBuf, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT manifest_path, manifest_digest FROM runtime_manifests WHERE runtime_id = ?1",
        )?;
        let mut rows = stmt.query(params![runtime_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some((
                std::path::PathBuf::from(row.get::<_, String>(0)?),
                row.get(1)?,
            )))
        } else {
            Ok(None)
        }
    }

    /// Lists runtime manifests no longer referenced by any retained generation.
    pub fn unreferenced_runtime_manifests(&self) -> Result<Vec<(String, std::path::PathBuf)>> {
        let mut stmt = self.conn.prepare(
            "SELECT runtime_id, manifest_path
             FROM runtime_manifests
             WHERE NOT EXISTS (
                 SELECT 1 FROM generation_runtimes
                 WHERE generation_runtimes.runtime_id = runtime_manifests.runtime_id
             )
             ORDER BY runtime_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                std::path::PathBuf::from(row.get::<_, String>(1)?),
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Database)
    }

    /// Removes a runtime manifest row after its managed files were removed.
    pub fn remove_runtime_manifest(&self, runtime_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM runtime_manifests WHERE runtime_id = ?1
             AND NOT EXISTS (
                 SELECT 1 FROM generation_runtimes
                 WHERE generation_runtimes.runtime_id = runtime_manifests.runtime_id
             )",
            params![runtime_id],
        )?;
        Ok(())
    }

    /// Returns realized store roots referenced by persisted runtime manifests.
    /// A malformed manifest is surfaced so GC fails closed instead of deleting
    /// an object whose runtime reachability cannot be proven.
    pub fn runtime_store_references(
        &self,
        runtime_root: &Path,
        store_root: &Path,
    ) -> Result<Vec<std::path::PathBuf>> {
        let mut stmt = self
            .conn
            .prepare("SELECT runtime_id, manifest_path, manifest_digest FROM runtime_manifests")?;
        let paths = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut references = Vec::new();
        for row in paths {
            let (runtime_id, manifest_path, expected_digest) = row?;
            let path = std::path::PathBuf::from(manifest_path);
            if !path.starts_with(runtime_root)
                || !std::fs::symlink_metadata(&path)
                    .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "runtime manifest is outside the managed runtime root: {} ({runtime_id})",
                    path.display()
                )));
            }
            let manifest: crate::domain::contracts::RuntimeManifest =
                serde_json::from_slice(&std::fs::read(&path)?)?;
            if crate::domain::contracts::digest_serialized(&manifest) != expected_digest {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "runtime manifest digest mismatch: {} ({runtime_id})",
                    path.display()
                )));
            }
            for reference in manifest.references {
                let reference = std::path::PathBuf::from(reference);
                let Ok(relative) = reference.strip_prefix(store_root) else {
                    // Host providers are validated at launch and are not
                    // package-owned GC roots.
                    continue;
                };
                let mut components = relative.components();
                let Some(std::path::Component::Normal(store_id)) = components.next() else {
                    return Err(Error::TransactionRecoveryRequired(format!(
                        "runtime reference is not a store object: {}",
                        reference.display()
                    )));
                };
                if components.next().is_some() {
                    return Err(Error::TransactionRecoveryRequired(format!(
                        "runtime reference is not a store root: {}",
                        reference.display()
                    )));
                }
                let store_id = store_id.to_str().ok_or_else(|| {
                    Error::TransactionRecoveryRequired(format!(
                        "runtime reference has a non-UTF-8 store id: {}",
                        reference.display()
                    ))
                })?;
                crate::store::StoreLayout::validate_component(store_id, "store id")?;
                references.push(store_root.join(store_id));
            }
        }
        Ok(references)
    }

    /// Returns the active generation for a profile.
    pub fn active_generation(&self, profile: &str) -> Result<Option<GenerationRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT profile, generation_id, previous_generation, manifest_path, created_at, active
             FROM generations WHERE profile = ?1 AND active = 1 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![profile])?;
        if let Some(row) = rows.next()? {
            Ok(Some(GenerationRecord {
                profile: row.get(0)?,
                generation_id: row.get(1)?,
                previous_generation: row.get(2)?,
                manifest_path: std::path::PathBuf::from(row.get::<_, String>(3)?),
                created_at: row.get(4)?,
                active: row.get::<_, i64>(5)? != 0,
            }))
        } else {
            Ok(None)
        }
    }

    /// Lists all generations, oldest first, for retention and diagnostics.
    pub fn list_generations(&self, profile: &str) -> Result<Vec<GenerationRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT profile, generation_id, previous_generation, manifest_path, created_at, active
             FROM generations WHERE profile = ?1 ORDER BY created_at ASC, generation_id ASC",
        )?;
        let rows = stmt.query_map(params![profile], |row| {
            Ok(GenerationRecord {
                profile: row.get(0)?,
                generation_id: row.get(1)?,
                previous_generation: row.get(2)?,
                manifest_path: std::path::PathBuf::from(row.get::<_, String>(3)?),
                created_at: row.get(4)?,
                active: row.get::<_, i64>(5)? != 0,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Database)
    }

    /// Detaches a store object from retained generation roots after an
    /// explicit removal plan.  Other package/profile references are checked
    /// separately by `store_is_referenced`.
    pub fn remove_generation_store_references(&self, store_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM generation_objects WHERE store_id = ?1",
            params![store_id],
        )?;
        Ok(())
    }

    /// Restores package and activation rows captured for a generation.
    pub fn restore_generation_state(&self, profile: &str, generation_id: &str) -> Result<()> {
        self.conn.execute("BEGIN IMMEDIATE", [])?;
        let result = (|| -> Result<()> {
            let exists: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM generations WHERE profile = ?1 AND generation_id = ?2)",
                params![profile, generation_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(Error::PackageNotFound(format!(
                    "generation {generation_id}"
                )));
            }
            let mut package_rows = Vec::new();
            {
                let mut stmt = self.conn.prepare(
                    "SELECT name, version, store_id, installed_at FROM generation_packages WHERE profile = ?1 AND generation_id = ?2",
                )?;
                let rows = stmt.query_map(params![profile, generation_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
                for row in rows {
                    package_rows.push(row?);
                }
            }
            let mut activation_rows = Vec::new();
            {
                let mut stmt = self.conn.prepare(
                    "SELECT command, store_id, package_name, target_path FROM generation_activations WHERE profile = ?1 AND generation_id = ?2",
                )?;
                let rows = stmt.query_map(params![profile, generation_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
                for row in rows {
                    activation_rows.push(row?);
                }
            }
            for (_, _, store_id, _) in &package_rows {
                let exists: bool = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM store_objects WHERE store_id = ?1)",
                    params![store_id],
                    |row| row.get(0),
                )?;
                if !exists {
                    return Err(Error::TransactionRecoveryRequired(format!(
                        "Store object {store_id} required by generation is missing"
                    )));
                }
            }
            self.conn.execute(
                "DELETE FROM activations WHERE profile = ?1",
                params![profile],
            )?;
            self.conn
                .execute("DELETE FROM packages WHERE profile = ?1", params![profile])?;
            for (name, version, store_id, installed_at) in package_rows {
                self.conn.execute(
                    "INSERT INTO packages (profile, name, active_version, active_store_id, installed_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![profile, name, version, store_id, installed_at],
                )?;
            }
            for (command, store_id, package_name, target_path) in activation_rows {
                self.conn.execute(
                    "INSERT INTO activations (profile, command, store_id, package_name, target_path) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![profile, command, store_id, package_name, target_path],
                )?;
            }
            self.conn.execute(
                "UPDATE generations SET active = 0 WHERE profile = ?1",
                params![profile],
            )?;
            self.conn.execute(
                "UPDATE generations SET active = 1 WHERE profile = ?1 AND generation_id = ?2",
                params![profile, generation_id],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute("COMMIT", [])?;
                Ok(())
            }
            Err(error) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(error)
            }
        }
    }

    /// Returns recorded activation links and their expected store targets.
    pub fn activation_targets(&self) -> Result<Vec<(std::path::PathBuf, std::path::PathBuf)>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.target_path, s.store_path, f.relative_path
             FROM activations a
             JOIN store_objects s ON s.store_id = a.store_id
             JOIN installed_files f ON f.store_id = s.store_id
             WHERE f.relative_path = '.pkg-launcher/' || a.command || '-native'
                OR f.relative_path = '.pkg-launcher/' || a.command
                OR f.relative_path = 'bin/' || a.command
                OR f.relative_path = 'usr/bin/' || a.command
             ORDER BY CASE
                WHEN f.relative_path = '.pkg-launcher/' || a.command || '-native' THEN 0
                WHEN f.relative_path = '.pkg-launcher/' || a.command THEN 1
                ELSE 2
             END",
        )?;
        let rows = stmt.query_map([], |row| {
            let link: String = row.get(0)?;
            let root: String = row.get(1)?;
            let relative: String = row.get(2)?;
            Ok((
                std::path::PathBuf::from(link),
                std::path::PathBuf::from(root).join(relative),
            ))
        })?;
        let mut result = Vec::new();
        let mut seen_links = std::collections::HashSet::new();
        for row in rows {
            let (link, target) = row?;
            if seen_links.insert(link.clone()) {
                result.push((link, target));
            }
        }
        Ok(result)
    }

    /// Looks up an installed package by name in a profile.
    pub fn get_package(&self, profile: &str, name: &str) -> Result<Option<InstalledPackage>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.name, p.active_version, s.architecture, s.format, s.digest, p.active_store_id, s.store_path, p.installed_at
             FROM packages p
             JOIN store_objects s ON p.active_store_id = s.store_id
             WHERE p.profile = ?1 AND p.name = ?2",
        )?;

        let mut rows = stmt.query(params![profile, name])?;
        if let Some(row) = rows.next()? {
            let pkg_name: String = row.get(0)?;
            let ver: String = row.get(1)?;
            let arch: String = row.get(2)?;
            let fmt_str: String = row.get(3)?;
            let dig: String = row.get(4)?;
            let store_id: String = row.get(5)?;
            let store_path: String = row.get(6)?;
            let installed_at: String = row.get(7)?;

            let binaries = self.get_activated_binaries(profile, &pkg_name)?;

            let format = match fmt_str.as_str() {
                "deb" => PackageFormat::Deb,
                "rpm" => PackageFormat::Rpm,
                "alpm" => PackageFormat::Alpm,
                _ => PackageFormat::Tarball,
            };

            let digest_parts: Vec<&str> = dig.split(':').collect();
            let digest = if digest_parts.len() == 2 {
                ArtifactDigest::new(digest_parts[0], digest_parts[1])
            } else {
                ArtifactDigest::sha256(dig)
            };

            Ok(Some(InstalledPackage {
                name: PackageName::new(pkg_name)?,
                version: PackageVersion::new(ver),
                architecture: Architecture::parse(&arch),
                format,
                digest,
                store_id,
                store_path: std::path::PathBuf::from(store_path),
                installed_at,
                active: true,
                profile: profile.to_string(),
                binaries,
            }))
        } else {
            Ok(None)
        }
    }

    /// Lists all installed packages in a profile.
    pub fn list_packages(&self, profile: &str) -> Result<Vec<InstalledPackage>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.name, p.active_version, s.architecture, s.format, s.digest, p.active_store_id, s.store_path, p.installed_at
             FROM packages p
             JOIN store_objects s ON p.active_store_id = s.store_id
             WHERE p.profile = ?1
             ORDER BY p.name ASC",
        )?;

        let rows = stmt.query_map(params![profile], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?;

        let mut result = Vec::new();
        for r in rows {
            let (pkg_name, ver, arch, fmt_str, dig, store_id, store_path, installed_at) = r?;
            let binaries = self.get_activated_binaries(profile, &pkg_name)?;

            let format = match fmt_str.as_str() {
                "deb" => PackageFormat::Deb,
                "rpm" => PackageFormat::Rpm,
                "alpm" => PackageFormat::Alpm,
                _ => PackageFormat::Tarball,
            };

            let digest_parts: Vec<&str> = dig.split(':').collect();
            let digest = if digest_parts.len() == 2 {
                ArtifactDigest::new(digest_parts[0], digest_parts[1])
            } else {
                ArtifactDigest::sha256(dig)
            };

            result.push(InstalledPackage {
                name: PackageName::new(pkg_name)?,
                version: PackageVersion::new(ver),
                architecture: Architecture::parse(&arch),
                format,
                digest,
                store_id,
                store_path: std::path::PathBuf::from(store_path),
                installed_at,
                active: true,
                profile: profile.to_string(),
                binaries,
            });
        }

        Ok(result)
    }

    /// Returns profile names represented in committed package/generation state.
    pub fn list_profiles(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT profile FROM packages
             UNION SELECT profile FROM activations
             UNION SELECT profile FROM integrations
             UNION SELECT profile FROM generations
             ORDER BY profile ASC",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Database)
    }

    /// Deletes all committed state belonging to a profile in one transaction.
    /// Store objects are intentionally retained for garbage collection because
    /// they may still be shared by another profile or retained generation.
    pub fn remove_profile_state(&self, profile: &str) -> Result<()> {
        self.conn.execute("BEGIN IMMEDIATE", [])?;
        let result = (|| -> Result<()> {
            self.conn.execute(
                "DELETE FROM activations WHERE profile = ?1",
                params![profile],
            )?;
            self.conn.execute(
                "DELETE FROM integrations WHERE profile = ?1",
                params![profile],
            )?;
            self.conn
                .execute("DELETE FROM packages WHERE profile = ?1", params![profile])?;
            self.conn.execute(
                "DELETE FROM generations WHERE profile = ?1",
                params![profile],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute("COMMIT", [])?;
                Ok(())
            }
            Err(error) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(error)
            }
        }
    }

    /// Reconstructs the normalized package facts needed by the resolver for
    /// packages already installed in a profile. Dependency metadata is not
    /// retroactively invented; capabilities come only from recorded payload
    /// paths, and unresolved source constraints remain absent.
    pub fn normalized_packages_for_profile(&self, profile: &str) -> Result<Vec<NormalizedPackage>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.name, p.active_version, s.architecture, s.format, s.digest,
                    s.store_id, s.store_path
             FROM packages p
             JOIN store_objects s ON p.active_store_id = s.store_id
             WHERE p.profile = ?1
             ORDER BY p.name ASC",
        )?;
        let rows = stmt.query_map(params![profile], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;

        let mut packages = Vec::new();
        for row in rows {
            let (name, version, arch, format, digest, store_id, store_path) = row?;
            let mut files_stmt = self.conn.prepare(
                "SELECT relative_path FROM installed_files WHERE store_id = ?1 ORDER BY relative_path ASC",
            )?;
            let file_paths = files_stmt
                .query_map(params![store_id], |file| file.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            let mut entries = Vec::new();
            let mut provides = Vec::new();
            for relative in file_paths {
                let relative_path = std::path::PathBuf::from(&relative);
                let full_path = std::path::Path::new(&store_path).join(&relative_path);
                let metadata = std::fs::symlink_metadata(&full_path).ok();
                let is_dir = metadata.as_ref().is_some_and(|m| m.is_dir());
                let is_symlink = metadata
                    .as_ref()
                    .is_some_and(|m| m.file_type().is_symlink());
                let is_bin = relative_path.parent().is_some_and(|parent| {
                    parent == std::path::Path::new("bin")
                        || parent == std::path::Path::new("usr/bin")
                });
                if is_bin && let Some(command) = relative_path.file_name().and_then(|n| n.to_str())
                {
                    provides.push(Capability::Executable(command.to_string()));
                }
                if let Some(file_name) = relative_path.file_name().and_then(|n| n.to_str())
                    && file_name.contains(".so")
                {
                    provides.push(Capability::SharedLibrary(file_name.to_string()));
                }
                entries.push(PackageEntry {
                    relative_path,
                    is_dir,
                    is_symlink,
                    symlink_target: if is_symlink {
                        std::fs::read_link(&full_path).ok()
                    } else {
                        None
                    },
                    mode: 0,
                    size: 0,
                });
            }

            let package_format = match format.as_str() {
                "deb" => PackageFormat::Deb,
                "rpm" => PackageFormat::Rpm,
                "alpm" => PackageFormat::Alpm,
                _ => PackageFormat::Tarball,
            };
            let digest = if let Some((algorithm, hex)) = digest.split_once(':') {
                ArtifactDigest::new(algorithm, hex)
            } else {
                ArtifactDigest::sha256(digest)
            };
            packages.push(NormalizedPackage {
                name: PackageName::new(name)?,
                version: PackageVersion::new(version),
                architecture: Architecture::parse(&arch),
                format: package_format,
                digest,
                size_bytes: 0,
                description: None,
                dependencies: Vec::new(),
                constraints: Vec::new(),
                provides,
                versioned_provides: Vec::new(),
                scripts: Vec::<LifecycleScript>::new(),
                entries,
                installed_size: None,
            });
        }
        Ok(packages)
    }

    /// Reconstructs normalized candidates from the immutable repository
    /// snapshot.  The catalog contains dependency/provides evidence but not a
    /// payload tree; ELF and realization facts are added only after acquisition.
    pub fn normalized_remote_packages(&self) -> Result<Vec<NormalizedPackage>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT name, version, architecture, format, digest, size_bytes,
                    {}
             FROM remote_packages ORDER BY name ASC, version ASC",
            self.remote_columns()
        ))?;
        let rows = stmt.query_map([], |row| {
            let format = match row.get::<_, String>(3)?.as_str() {
                "deb" => PackageFormat::Deb,
                "rpm" => PackageFormat::Rpm,
                "alpm" => PackageFormat::Alpm,
                _ => PackageFormat::Tarball,
            };
            let digest_raw: String = row.get(4)?;
            let digest = digest_raw
                .split_once(':')
                .map(|(algorithm, hex)| ArtifactDigest::new(algorithm, hex))
                .unwrap_or_else(|| ArtifactDigest::sha256(digest_raw));
            Ok(NormalizedPackage {
                name: PackageName::new(row.get::<_, String>(0)?).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                version: PackageVersion::new(row.get::<_, String>(1)?),
                architecture: Architecture::parse(&row.get::<_, String>(2)?),
                format,
                digest,
                size_bytes: row.get(5)?,
                description: None,
                dependencies: Vec::new(),
                constraints: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
                provides: serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default(),
                versioned_provides: serde_json::from_str(&row.get::<_, String>(8)?)
                    .unwrap_or_default(),
                scripts: Vec::new(),
                entries: Vec::new(),
                installed_size: None,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Database)
    }

    /// Returns list of commands activated in a profile for a given package.
    pub fn get_activated_binaries(&self, profile: &str, package_name: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT command FROM activations WHERE profile = ?1 AND package_name = ?2 ORDER BY command ASC",
        )?;
        let rows = stmt.query_map(params![profile, package_name], |row| row.get(0))?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Records a host integration action and its ownership evidence.
    pub fn record_integration(&self, record: &IntegrationRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO integrations (profile, package_name, store_id, kind, source_path, target_path, source_digest, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(profile, target_path) DO UPDATE SET package_name=excluded.package_name,
                store_id=excluded.store_id, kind=excluded.kind, source_path=excluded.source_path,
                source_digest=excluded.source_digest, created_at=excluded.created_at",
            params![
                record.profile,
                record.package_name,
                record.store_id,
                record.kind,
                record.source_path.to_string_lossy(),
                record.target_path.to_string_lossy(),
                record.source_digest,
                chrono_now(),
            ],
        )?;
        Ok(())
    }

    /// Lists integrations owned by a profile/package, ordered by target.
    pub fn list_integrations(
        &self,
        profile: &str,
        package_name: Option<&str>,
    ) -> Result<Vec<IntegrationRecord>> {
        let mut records = Vec::new();
        if let Some(package_name) = package_name {
            let mut stmt = self.conn.prepare(
                "SELECT profile, package_name, store_id, kind, source_path, target_path, source_digest
                 FROM integrations WHERE profile = ?1 AND package_name = ?2 ORDER BY target_path",
            )?;
            let rows = stmt.query_map(params![profile, package_name], |row| {
                Ok(IntegrationRecord {
                    profile: row.get(0)?,
                    package_name: row.get(1)?,
                    store_id: row.get(2)?,
                    kind: row.get(3)?,
                    source_path: std::path::PathBuf::from(row.get::<_, String>(4)?),
                    target_path: std::path::PathBuf::from(row.get::<_, String>(5)?),
                    source_digest: row.get(6)?,
                })
            })?;
            for row in rows {
                records.push(row?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT profile, package_name, store_id, kind, source_path, target_path, source_digest
                 FROM integrations WHERE profile = ?1 ORDER BY target_path",
            )?;
            let rows = stmt.query_map(params![profile], |row| {
                Ok(IntegrationRecord {
                    profile: row.get(0)?,
                    package_name: row.get(1)?,
                    store_id: row.get(2)?,
                    kind: row.get(3)?,
                    source_path: std::path::PathBuf::from(row.get::<_, String>(4)?),
                    target_path: std::path::PathBuf::from(row.get::<_, String>(5)?),
                    source_digest: row.get(6)?,
                })
            })?;
            for row in rows {
                records.push(row?);
            }
        }
        Ok(records)
    }

    /// Deletes one integration ownership record after its target was handled.
    pub fn remove_integration(&self, profile: &str, target_path: &Path) -> Result<()> {
        self.conn.execute(
            "DELETE FROM integrations WHERE profile = ?1 AND target_path = ?2",
            params![profile, target_path.to_string_lossy()],
        )?;
        Ok(())
    }

    /// Removes a package and its activations from a profile.
    pub fn remove_package_from_profile(&self, profile: &str, package_name: &str) -> Result<()> {
        self.conn.execute("BEGIN IMMEDIATE", [])?;
        let result = (|| -> Result<()> {
            self.conn.execute(
                "DELETE FROM activations WHERE profile = ?1 AND package_name = ?2",
                params![profile, package_name],
            )?;
            self.conn.execute(
                "DELETE FROM packages WHERE profile = ?1 AND name = ?2",
                params![profile, package_name],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute("COMMIT", [])?;
                Ok(())
            }
            Err(error) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(error)
            }
        }
    }

    /// Marks a store object as unreachable and optionally deletes its record.
    pub fn store_is_referenced(&self, store_id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM packages WHERE active_store_id = ?1
                UNION ALL
                SELECT 1 FROM activations WHERE store_id = ?1
                UNION ALL
                SELECT 1 FROM generation_objects WHERE store_id = ?1
                UNION ALL
                SELECT 1 FROM integrations WHERE store_id = ?1
                UNION ALL
                SELECT 1 FROM transactions
                 WHERE store_id = ?1 AND phase NOT IN ('Completed', 'FailedClean')
            )",
            params![store_id],
            |r| r.get(0),
        )?)
    }

    /// Checks references that belong to committed package/profile state.
    ///
    /// Recovery uses this variant while it is reconciling one incomplete
    /// transaction: that transaction's own store id must not keep its orphan
    /// staging result alive.
    pub fn store_is_referenced_by_committed_state(&self, store_id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM packages WHERE active_store_id = ?1
                UNION ALL
                SELECT 1 FROM activations WHERE store_id = ?1
                UNION ALL
                SELECT 1 FROM generation_objects WHERE store_id = ?1
                UNION ALL
                SELECT 1 FROM integrations WHERE store_id = ?1
            )",
            params![store_id],
            |r| r.get(0),
        )?)
    }

    /// Lists all store objects known to the state database.
    pub fn list_store_objects(&self) -> Result<Vec<StoreObjectRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT store_id, package_name, version, architecture, format, digest, store_path, installed_at
             FROM store_objects ORDER BY store_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(StoreObjectRecord {
                store_id: row.get(0)?,
                package_name: row.get(1)?,
                version: row.get(2)?,
                architecture: row.get(3)?,
                format: row.get(4)?,
                digest: row.get(5)?,
                store_path: std::path::PathBuf::from(row.get::<_, String>(6)?),
                installed_at: row.get(7)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Database)
    }

    /// Returns the recorded payload target for an activation owned by this package.
    pub fn activation_target(
        &self,
        profile: &str,
        command: &str,
        package: &str,
    ) -> Result<Option<std::path::PathBuf>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.store_path, f.relative_path
             FROM activations a
             JOIN store_objects s ON s.store_id = a.store_id
             JOIN installed_files f ON f.store_id = s.store_id
             WHERE a.profile = ?1 AND a.command = ?2 AND a.package_name = ?3
               AND (f.relative_path = '.pkg-launcher/' || ?2 || '-native'
                    OR f.relative_path = '.pkg-launcher/' || ?2
                    OR f.relative_path = 'bin/' || ?2
                    OR f.relative_path = 'usr/bin/' || ?2)
             ORDER BY CASE
                WHEN f.relative_path = '.pkg-launcher/' || ?2 || '-native' THEN 0
                WHEN f.relative_path = '.pkg-launcher/' || ?2 THEN 1
                ELSE 2
             END
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![profile, command, package])?;
        if let Some(row) = rows.next()? {
            let root: String = row.get(0)?;
            let relative: String = row.get(1)?;
            Ok(Some(std::path::PathBuf::from(root).join(relative)))
        } else {
            Ok(None)
        }
    }

    /// Deletes an unreferenced store record.
    pub fn remove_store_object(&self, store_id: &str) -> Result<()> {
        if self.store_is_referenced(store_id)? {
            return Err(Error::Internal(format!(
                "Store object is still referenced: {store_id}"
            )));
        }
        self.conn.execute(
            "DELETE FROM store_objects WHERE store_id = ?1",
            params![store_id],
        )?;
        Ok(())
    }

    /// Deletes an object record while reconciling the incomplete transaction
    /// that owns it. Callers must have checked committed ownership first.
    pub fn remove_store_object_after_recovery(&self, store_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM store_objects WHERE store_id = ?1",
            params![store_id],
        )?;
        Ok(())
    }

    /// Deletes an object after the caller has revalidated all references and
    /// intentionally excluded its own active mutation transaction.
    pub fn remove_store_object_after_committed_check(&self, store_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM store_objects WHERE store_id = ?1",
            params![store_id],
        )?;
        Ok(())
    }
}

/// Helper returning current timestamp in RFC 3339 / ISO 8601 format.
fn chrono_now() -> String {
    // Generate ISO 8601 timestamp without extra chrono dependency
    use std::time::SystemTime;
    let now = SystemTime::now();
    let since_epoch = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", since_epoch.as_secs())
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    // Table/column names are fixed internal values at all call sites; using a
    // quoted identifier keeps the read-only compatibility probe deterministic.
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_initialization_and_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("state.db");
        let db = StateDatabase::open(&db_path).unwrap();

        let pkg_name = PackageName::new("testpkg").unwrap();
        let version = PackageVersion::new("1.0.0");
        let arch = Architecture::X86_64;
        let digest = ArtifactDigest::sha256("1234567890abcdef");
        let store_path = temp_dir.path().join("store/obj1");

        db.record_store_object(&NewStoreObject {
            store_id: "obj1",
            name: &pkg_name,
            version: &version,
            architecture: &arch,
            format: PackageFormat::Deb,
            digest: &digest,
            store_path: &store_path,
            files: &[std::path::PathBuf::from("usr/bin/testcmd")],
        })
        .unwrap();

        db.record_activation(
            "default",
            "testcmd",
            "obj1",
            "testpkg",
            &store_path.join("usr/bin/testcmd"),
        )
        .unwrap();

        db.record_package("default", &pkg_name, &version, "obj1")
            .unwrap();

        let pkg = db.get_package("default", "testpkg").unwrap().unwrap();
        assert_eq!(pkg.name.as_str(), "testpkg");
        assert_eq!(pkg.version.as_str(), "1.0.0");
        assert_eq!(pkg.binaries, vec!["testcmd".to_string()]);

        let list = db.list_packages("default").unwrap();
        assert_eq!(list.len(), 1);

        // Check conflict detection
        let conflict = db
            .find_conflicting_activation("default", "testcmd", "otherpkg")
            .unwrap();
        assert_eq!(conflict, Some("testpkg".to_string()));

        let no_conflict = db
            .find_conflicting_activation("default", "testcmd", "testpkg")
            .unwrap();
        assert_eq!(no_conflict, None);
    }

    #[test]
    fn read_only_open_supports_pre_metadata_catalogs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("legacy.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE remote_packages (
                repository_id TEXT NOT NULL,
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                architecture TEXT NOT NULL,
                format TEXT NOT NULL,
                digest TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                url TEXT NOT NULL
            );
            INSERT INTO remote_packages VALUES ('repo', 'tool', '1.0', 'x86_64', 'deb', 'abc', 3, 'file:///tool.deb');",
        )
        .unwrap();
        drop(conn);
        let db = StateDatabase::open_read_only(&db_path).unwrap();
        let package = db.get_remote_package("tool").unwrap().unwrap();
        assert!(package.constraints.is_empty());
        assert!(package.provides.is_empty());
    }

    #[test]
    fn remote_snapshot_round_trips_versioned_provides() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = StateDatabase::open(&temp_dir.path().join("state.db")).unwrap();
        let provider = crate::domain::package::RemotePackage {
            repository_id: "repo".into(),
            name: "virtual-provider".into(),
            version: "1.0-1".into(),
            architecture: "x86_64".into(),
            format: "rpm".into(),
            digest: "a".repeat(64),
            size_bytes: 10,
            url: "https://example.invalid/provider.rpm".into(),
            constraints: Vec::new(),
            provides: Vec::new(),
            versioned_provides: vec![crate::domain::package::VersionedCapability {
                capability: Capability::Feature("virtual-api".into()),
                version: PackageVersion::new("2.4"),
            }],
        };
        db.commit_repository_snapshot("repo", "rpm", "https://example.invalid", "1", &[provider])
            .unwrap();

        let loaded = db.get_remote_package("virtual-provider").unwrap().unwrap();
        assert_eq!(loaded.versioned_provides.len(), 1);
        assert_eq!(loaded.versioned_provides[0].version.as_str(), "2.4");
        let normalized = db.normalized_remote_packages().unwrap();
        assert_eq!(normalized[0].versioned_provides, loaded.versioned_provides);
    }

    #[test]
    fn normalized_remote_packages_accepts_valid_rpm_name_spelling() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = StateDatabase::open(&temp_dir.path().join("state.db")).unwrap();
        let package = crate::domain::package::RemotePackage {
            repository_id: "fedora".into(),
            name: "0xFFFF".into(),
            version: "0.10-8.fc41".into(),
            architecture: "x86_64".into(),
            format: "rpm".into(),
            digest: "b".repeat(64),
            size_bytes: 10,
            url: "https://example.invalid/0xFFFF.rpm".into(),
            constraints: Vec::new(),
            provides: Vec::new(),
            versioned_provides: Vec::new(),
        };
        db.commit_repository_snapshot("fedora", "rpm", "https://example.invalid", "41", &[package])
            .unwrap();

        let normalized = db.normalized_remote_packages().unwrap();
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].name.as_str(), "0xFFFF");
    }
}
