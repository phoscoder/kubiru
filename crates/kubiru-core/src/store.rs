//! Local persistence for cluster settings and preferences.
//!
//! Everything lives in one SQLite file in the platform data directory. Tokens
//! are never written here; see [`crate::vault`].

use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, params};
use url::Url;

use crate::cluster::{Cluster, ClusterId, TlsVerification, ValidCluster};

/// Schema migrations, applied in order. The index + 1 is stored in
/// `PRAGMA user_version`, so never edit or reorder an existing entry.
const MIGRATIONS: &[&str] = &[r#"
    CREATE TABLE clusters (
        id             INTEGER PRIMARY KEY,
        name           TEXT NOT NULL UNIQUE COLLATE NOCASE,
        api_url        TEXT NOT NULL,
        description    TEXT NOT NULL DEFAULT '',
        tls_mode       TEXT NOT NULL CHECK (tls_mode IN ('system', 'custom_ca', 'insecure')),
        ca_certificate TEXT,
        created_at     TEXT NOT NULL,
        updated_at     TEXT NOT NULL
    );
    CREATE TABLE preferences (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
"#];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("a cluster named \u{201c}{0}\u{201d} already exists")]
    DuplicateName(String),
    #[error("the cluster no longer exists")]
    NotFound,
    #[error("could not prepare the data directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("the local database is newer than this version of Kubiru (schema {0})")]
    UnsupportedSchema(i64),
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored data is corrupt: {0}")]
    Corrupt(String),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// Where the database lives unless overridden.
pub fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "Kubiru", "Kubiru")
        .map(|dirs| dirs.data_dir().join("kubiru.sqlite3"))
}

#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the database at `path` and bring its schema
    /// up to date. On Unix the directory and file are made private to the user.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
            restrict_permissions(dir, 0o700)?;
        }
        let conn = Connection::open(path)?;
        restrict_permissions(path, 0o600)?;
        Self::init(conn)
    }

    /// A throwaway database, for tests and previews.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let current: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        let latest = MIGRATIONS.len() as i64;
        if current > latest {
            return Err(StoreError::UnsupportedSchema(current));
        }
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", index as i64 + 1)?;
            tx.commit()?;
        }
        Ok(())
    }

    /// All clusters, alphabetically.
    pub fn clusters(&self) -> Result<Vec<Cluster>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, api_url, description, tls_mode, ca_certificate, created_at, updated_at
             FROM clusters ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], read_cluster)?;
        rows.map(|row| row?.map_err(StoreError::Corrupt)).collect()
    }

    pub fn cluster(&self, id: ClusterId) -> Result<Cluster> {
        self.conn
            .query_row(
                "SELECT id, name, api_url, description, tls_mode, ca_certificate, created_at, updated_at
                 FROM clusters WHERE id = ?1",
                [id.0],
                read_cluster,
            )
            .optional()?
            .ok_or(StoreError::NotFound)?
            .map_err(StoreError::Corrupt)
    }

    pub fn insert_cluster(&self, cluster: &ValidCluster) -> Result<Cluster> {
        let now = now();
        let (mode, ca) = tls_columns(&cluster.tls);
        self.conn
            .execute(
                "INSERT INTO clusters (name, api_url, description, tls_mode, ca_certificate, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![cluster.name, cluster.api_url.as_str(), cluster.description, mode, ca, now],
            )
            .map_err(|e| unique_violation(e, &cluster.name))?;
        self.cluster(ClusterId(self.conn.last_insert_rowid()))
    }

    pub fn update_cluster(&self, id: ClusterId, cluster: &ValidCluster) -> Result<Cluster> {
        let (mode, ca) = tls_columns(&cluster.tls);
        let changed = self
            .conn
            .execute(
                "UPDATE clusters
                 SET name = ?2, api_url = ?3, description = ?4, tls_mode = ?5, ca_certificate = ?6, updated_at = ?7
                 WHERE id = ?1",
                params![id.0, cluster.name, cluster.api_url.as_str(), cluster.description, mode, ca, now()],
            )
            .map_err(|e| unique_violation(e, &cluster.name))?;
        if changed == 0 {
            return Err(StoreError::NotFound);
        }
        self.cluster(id)
    }

    /// Remove a cluster and the preferences recorded for it.
    pub fn delete_cluster(&mut self, id: ClusterId) -> Result<()> {
        let tx = self.conn.transaction()?;
        let changed = tx.execute("DELETE FROM clusters WHERE id = ?1", [id.0])?;
        tx.execute(
            "DELETE FROM preferences WHERE key = ?1",
            [namespace_key(id)],
        )?;
        tx.execute(
            "DELETE FROM preferences WHERE key = ?1 AND value = ?2",
            params![keys::ACTIVE_CLUSTER, id.0.to_string()],
        )?;
        tx.commit()?;
        if changed == 0 {
            Err(StoreError::NotFound)
        } else {
            Ok(())
        }
    }

    pub fn preference(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM preferences WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_preference(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO preferences (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    pub fn remove_preference(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM preferences WHERE key = ?1", [key])?;
        Ok(())
    }

    /// The cluster that was active when the app last closed.
    pub fn active_cluster(&self) -> Result<Option<ClusterId>> {
        Ok(self
            .preference(keys::ACTIVE_CLUSTER)?
            .and_then(|v| v.parse().ok())
            .map(ClusterId))
    }

    pub fn set_active_cluster(&self, id: Option<ClusterId>) -> Result<()> {
        match id {
            Some(id) => self.set_preference(keys::ACTIVE_CLUSTER, &id.0.to_string()),
            None => self.remove_preference(keys::ACTIVE_CLUSTER),
        }
    }

    /// The namespace last chosen for a cluster; `None` means all namespaces.
    pub fn namespace(&self, id: ClusterId) -> Result<Option<String>> {
        self.preference(&namespace_key(id))
    }

    pub fn set_namespace(&self, id: ClusterId, namespace: Option<&str>) -> Result<()> {
        match namespace {
            Some(ns) => self.set_preference(&namespace_key(id), ns),
            None => self.remove_preference(&namespace_key(id)),
        }
    }
}

/// Preference keys shared with the UI.
pub mod keys {
    pub const ACTIVE_CLUSTER: &str = "active_cluster";
    pub const THEME: &str = "theme";
    pub const SIDEBAR_COLLAPSED: &str = "sidebar_collapsed";
}

fn namespace_key(id: ClusterId) -> String {
    format!("namespace:{id}")
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn tls_columns(tls: &TlsVerification) -> (&'static str, Option<&str>) {
    match tls {
        TlsVerification::System => ("system", None),
        TlsVerification::CustomCa(pem) => ("custom_ca", Some(pem.as_str())),
        TlsVerification::Insecure => ("insecure", None),
    }
}

/// Map a row, keeping malformed stored values as an inner error so one bad
/// row is reported as corruption rather than a generic SQLite failure.
fn read_cluster(row: &Row<'_>) -> rusqlite::Result<Result<Cluster, String>> {
    let id: i64 = row.get(0)?;
    let api_url: String = row.get(2)?;
    let mode: String = row.get(4)?;
    let ca: Option<String> = row.get(5)?;
    let api_url = match Url::parse(&api_url) {
        Ok(url) => url,
        Err(e) => return Ok(Err(format!("cluster {id} has an invalid address: {e}"))),
    };
    let tls = match (mode.as_str(), ca) {
        ("system", _) => TlsVerification::System,
        ("insecure", _) => TlsVerification::Insecure,
        ("custom_ca", Some(pem)) => TlsVerification::CustomCa(pem),
        _ => return Ok(Err(format!("cluster {id} has an invalid TLS mode"))),
    };
    Ok(Ok(Cluster {
        id: ClusterId(id),
        name: row.get(1)?,
        api_url,
        description: row.get(3)?,
        tls,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    }))
}

fn unique_violation(error: rusqlite::Error, name: &str) -> StoreError {
    match &error {
        rusqlite::Error::SqliteFailure(e, _) if e.code == ErrorCode::ConstraintViolation => {
            StoreError::DuplicateName(name.to_owned())
        }
        _ => StoreError::Sqlite(error),
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn restrict_permissions(_: &Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::parse_api_url;

    fn valid(name: &str) -> ValidCluster {
        ValidCluster {
            name: name.into(),
            api_url: parse_api_url("https://10.0.0.1:6443").unwrap(),
            description: String::new(),
            token: None,
            tls: TlsVerification::System,
        }
    }

    #[test]
    fn clusters_round_trip_and_sort_by_name() {
        let store = Store::open_in_memory().unwrap();
        store.insert_cluster(&valid("zeta")).unwrap();
        let alpha = store
            .insert_cluster(&ValidCluster {
                tls: TlsVerification::CustomCa("pem".into()),
                ..valid("Alpha")
            })
            .unwrap();
        let names: Vec<_> = store
            .clusters()
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Alpha", "zeta"]);
        assert_eq!(
            store.cluster(alpha.id).unwrap().tls,
            TlsVerification::CustomCa("pem".into())
        );
    }

    #[test]
    fn names_are_unique_ignoring_case() {
        let store = Store::open_in_memory().unwrap();
        store.insert_cluster(&valid("prod")).unwrap();
        let err = store.insert_cluster(&valid("PROD")).unwrap_err();
        assert!(matches!(err, StoreError::DuplicateName(_)));
    }

    #[test]
    fn updating_changes_fields_and_timestamp_column() {
        let store = Store::open_in_memory().unwrap();
        let c = store.insert_cluster(&valid("dev")).unwrap();
        let updated = store
            .update_cluster(
                c.id,
                &ValidCluster {
                    description: "Dev".into(),
                    ..valid("dev2")
                },
            )
            .unwrap();
        assert_eq!(updated.name, "dev2");
        assert_eq!(updated.description, "Dev");
        assert!(matches!(
            store.update_cluster(ClusterId(999), &valid("x")),
            Err(StoreError::NotFound)
        ));
    }

    #[test]
    fn deleting_a_cluster_forgets_its_preferences() {
        let mut store = Store::open_in_memory().unwrap();
        let c = store.insert_cluster(&valid("dev")).unwrap();
        store.set_active_cluster(Some(c.id)).unwrap();
        store.set_namespace(c.id, Some("kube-system")).unwrap();
        store.delete_cluster(c.id).unwrap();
        assert_eq!(store.active_cluster().unwrap(), None);
        assert_eq!(store.namespace(c.id).unwrap(), None);
        assert!(matches!(
            store.delete_cluster(c.id),
            Err(StoreError::NotFound)
        ));
    }

    #[test]
    fn preferences_upsert() {
        let store = Store::open_in_memory().unwrap();
        store.set_preference(keys::THEME, "dark").unwrap();
        store.set_preference(keys::THEME, "light").unwrap();
        assert_eq!(
            store.preference(keys::THEME).unwrap().as_deref(),
            Some("light")
        );
        store.remove_preference(keys::THEME).unwrap();
        assert_eq!(store.preference(keys::THEME).unwrap(), None);
    }

    #[test]
    fn reopening_a_file_keeps_data_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("kubiru.sqlite3");
        {
            let store = Store::open(&path).unwrap();
            store.insert_cluster(&valid("dev")).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.clusters().unwrap().len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(
            Store::init(conn),
            Err(StoreError::UnsupportedSchema(99))
        ));
    }
}
