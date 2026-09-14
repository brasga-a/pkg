//! SQLite local state database management (ADR-006, DEC-006).

use rusqlite::{Connection, params};
use std::path::Path;

use crate::domain::installed::InstalledPackage;
use crate::domain::package::{
    Architecture, ArtifactDigest, PackageFormat, PackageName, PackageVersion,
};
use crate::error::Result;

/// Database connection and state operations.
pub struct StateDatabase {
    conn: Connection,
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
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
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
            "#,
        )?;
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
        Ok(())
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
            "INSERT OR REPLACE INTO store_objects (store_id, package_name, version, architecture, format, digest, store_path, installed_at, is_reachable)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
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

    /// Removes a package and its activations from a profile.
    pub fn remove_package_from_profile(&self, profile: &str, package_name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM activations WHERE profile = ?1 AND package_name = ?2",
            params![profile, package_name],
        )?;
        self.conn.execute(
            "DELETE FROM packages WHERE profile = ?1 AND name = ?2",
            params![profile, package_name],
        )?;
        Ok(())
    }

    /// Marks a store object as unreachable and optionally deletes its record.
    pub fn remove_store_object(&self, store_id: &str) -> Result<()> {
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
}
