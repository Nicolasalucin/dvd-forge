//! SQLite persistence for recent projects, the transcoding cache and job
//! history. The default database lives in
//! `~/.local/share/org.dvdforge.desktop/dvdforge.db`.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const CRATE_NAME: &str = "dvdforge-store";
pub const APP_DIR: &str = "org.dvdforge.desktop";
pub const DB_FILE: &str = "dvdforge.db";

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("error de SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("no se pudo determinar el directorio de datos del usuario")]
    NoDataDir,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub id: i64,
    pub file_path: String,
    pub name: String,
    pub disc_kind: String,
    pub updated_at: String,
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecentProjectInput {
    pub file_path: String,
    pub name: String,
    pub disc_kind: String,
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactKey {
    pub source_sha256: String,
    pub trim_in_ms: u64,
    pub trim_out_ms: u64,
    pub target_profile: String,
    pub target_bitrate: u32,
    pub audio_params_hash: String,
}

impl ArtifactKey {
    pub fn cache_id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.source_sha256.as_bytes());
        hasher.update(self.trim_in_ms.to_be_bytes());
        hasher.update(self.trim_out_ms.to_be_bytes());
        hasher.update(self.target_profile.as_bytes());
        hasher.update(self.target_bitrate.to_be_bytes());
        hasher.update(self.audio_params_hash.as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CachedArtifact {
    pub artifact_path: String,
    pub duration_ms: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct JobHistoryEntry {
    pub id: String,
    pub project_name: String,
    pub disc_kind: String,
    pub iso_path: Option<String>,
    pub iso_size_bytes: u64,
    pub duration_seconds: f64,
    pub status: String,
    pub log_summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NewJobHistory {
    pub id: String,
    pub project_name: String,
    pub disc_kind: String,
    pub iso_path: Option<String>,
    pub iso_size_bytes: u64,
    pub duration_seconds: f64,
    pub status: String,
    pub log_summary: String,
}

#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let store = Self { path };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_default() -> Result<Self, StoreError> {
        Self::open(default_db_path()?)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("artifacts")
    }

    pub fn artifact_blob_path(&self, key: &ArtifactKey, extension: &str) -> PathBuf {
        self.artifacts_dir().join(format!(
            "{}.{}",
            key.cache_id(),
            extension.trim_start_matches('.')
        ))
    }

    fn connection(&self) -> Result<Connection, StoreError> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(connection)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        let connection = self.connection()?;
        connection.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS recent_projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                disc_kind TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                thumbnail_path TEXT
            );
            CREATE TABLE IF NOT EXISTS artifact_cache (
                source_sha256 TEXT NOT NULL,
                trim_in_ms INTEGER NOT NULL,
                trim_out_ms INTEGER NOT NULL,
                target_profile TEXT NOT NULL,
                target_bitrate INTEGER NOT NULL,
                audio_params_hash TEXT NOT NULL,
                artifact_path TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (
                    source_sha256, trim_in_ms, trim_out_ms,
                    target_profile, target_bitrate, audio_params_hash
                )
            );
            CREATE TABLE IF NOT EXISTS job_history (
                id TEXT PRIMARY KEY,
                project_name TEXT NOT NULL,
                disc_kind TEXT NOT NULL,
                iso_path TEXT,
                iso_size_bytes INTEGER NOT NULL DEFAULT 0,
                duration_seconds REAL NOT NULL DEFAULT 0,
                status TEXT NOT NULL,
                log_summary TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL
            );
            ",
        )?;
        Ok(())
    }

    pub fn record_recent(&self, input: &RecentProjectInput) -> Result<RecentProject, StoreError> {
        let connection = self.connection()?;
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "INSERT INTO recent_projects (file_path, name, disc_kind, updated_at, thumbnail_path)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(file_path) DO UPDATE SET
                name = excluded.name,
                disc_kind = excluded.disc_kind,
                updated_at = excluded.updated_at,
                thumbnail_path = excluded.thumbnail_path",
            params![
                input.file_path,
                input.name,
                input.disc_kind,
                now,
                input.thumbnail_path
            ],
        )?;
        let id = connection.query_row(
            "SELECT id FROM recent_projects WHERE file_path = ?1",
            params![input.file_path],
            |row| row.get(0),
        )?;
        Ok(RecentProject {
            id,
            file_path: input.file_path.clone(),
            name: input.name.clone(),
            disc_kind: input.disc_kind.clone(),
            updated_at: now,
            thumbnail_path: input.thumbnail_path.clone(),
        })
    }

    pub fn recent_projects(&self, limit: u32) -> Result<Vec<RecentProject>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, file_path, name, disc_kind, updated_at, thumbnail_path
             FROM recent_projects ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit.max(1)], |row| {
            Ok(RecentProject {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                disc_kind: row.get(3)?,
                updated_at: row.get(4)?,
                thumbnail_path: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn forget_recent(&self, file_path: &str) -> Result<(), StoreError> {
        let connection = self.connection()?;
        connection.execute(
            "DELETE FROM recent_projects WHERE file_path = ?1",
            params![file_path],
        )?;
        Ok(())
    }

    pub fn lookup_artifact(&self, key: &ArtifactKey) -> Result<Option<CachedArtifact>, StoreError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT artifact_path, duration_ms, created_at FROM artifact_cache
                 WHERE source_sha256 = ?1 AND trim_in_ms = ?2 AND trim_out_ms = ?3
                   AND target_profile = ?4 AND target_bitrate = ?5 AND audio_params_hash = ?6",
                params![
                    key.source_sha256,
                    key.trim_in_ms as i64,
                    key.trim_out_ms as i64,
                    key.target_profile,
                    key.target_bitrate as i64,
                    key.audio_params_hash
                ],
                |row| {
                    Ok(CachedArtifact {
                        artifact_path: row.get(0)?,
                        duration_ms: row.get::<_, i64>(1)?.max(0) as u64,
                        created_at: row.get(2)?,
                    })
                },
            )
            .optional()?;
        if let Some(artifact) = &row {
            if !Path::new(&artifact.artifact_path).is_file() {
                self.remove_artifact(key)?;
                return Ok(None);
            }
        }
        Ok(row)
    }

    pub fn put_artifact(
        &self,
        key: &ArtifactKey,
        artifact_path: &Path,
        duration_ms: u64,
    ) -> Result<CachedArtifact, StoreError> {
        let connection = self.connection()?;
        let now = Utc::now().to_rfc3339();
        let path = artifact_path.display().to_string();
        connection.execute(
            "INSERT INTO artifact_cache (
                source_sha256, trim_in_ms, trim_out_ms, target_profile,
                target_bitrate, audio_params_hash, artifact_path, duration_ms, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(source_sha256, trim_in_ms, trim_out_ms, target_profile,
                         target_bitrate, audio_params_hash)
             DO UPDATE SET artifact_path = excluded.artifact_path,
                           duration_ms = excluded.duration_ms,
                           created_at = excluded.created_at",
            params![
                key.source_sha256,
                key.trim_in_ms as i64,
                key.trim_out_ms as i64,
                key.target_profile,
                key.target_bitrate as i64,
                key.audio_params_hash,
                path,
                duration_ms as i64,
                now
            ],
        )?;
        Ok(CachedArtifact {
            artifact_path: path,
            duration_ms,
            created_at: now,
        })
    }

    pub fn remove_artifact(&self, key: &ArtifactKey) -> Result<(), StoreError> {
        let connection = self.connection()?;
        connection.execute(
            "DELETE FROM artifact_cache
             WHERE source_sha256 = ?1 AND trim_in_ms = ?2 AND trim_out_ms = ?3
               AND target_profile = ?4 AND target_bitrate = ?5 AND audio_params_hash = ?6",
            params![
                key.source_sha256,
                key.trim_in_ms as i64,
                key.trim_out_ms as i64,
                key.target_profile,
                key.target_bitrate as i64,
                key.audio_params_hash
            ],
        )?;
        Ok(())
    }

    pub fn record_job(&self, job: &NewJobHistory) -> Result<JobHistoryEntry, StoreError> {
        let connection = self.connection()?;
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "INSERT INTO job_history (
                id, project_name, disc_kind, iso_path, iso_size_bytes,
                duration_seconds, status, log_summary, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                iso_path = excluded.iso_path,
                iso_size_bytes = excluded.iso_size_bytes,
                duration_seconds = excluded.duration_seconds,
                status = excluded.status,
                log_summary = excluded.log_summary",
            params![
                job.id,
                job.project_name,
                job.disc_kind,
                job.iso_path,
                job.iso_size_bytes as i64,
                job.duration_seconds,
                job.status,
                job.log_summary,
                now
            ],
        )?;
        Ok(JobHistoryEntry {
            id: job.id.clone(),
            project_name: job.project_name.clone(),
            disc_kind: job.disc_kind.clone(),
            iso_path: job.iso_path.clone(),
            iso_size_bytes: job.iso_size_bytes,
            duration_seconds: job.duration_seconds,
            status: job.status.clone(),
            log_summary: job.log_summary.clone(),
            created_at: now,
        })
    }

    pub fn job_history(&self, limit: u32) -> Result<Vec<JobHistoryEntry>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, project_name, disc_kind, iso_path, iso_size_bytes,
                    duration_seconds, status, log_summary, created_at
             FROM job_history ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit.max(1)], |row| {
            Ok(JobHistoryEntry {
                id: row.get(0)?,
                project_name: row.get(1)?,
                disc_kind: row.get(2)?,
                iso_path: row.get(3)?,
                iso_size_bytes: row.get::<_, i64>(4)?.max(0) as u64,
                duration_seconds: row.get(5)?,
                status: row.get(6)?,
                log_summary: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

pub fn data_root() -> Result<PathBuf, StoreError> {
    if let Ok(value) = std::env::var("XDG_DATA_HOME") {
        if !value.trim().is_empty() {
            return Ok(PathBuf::from(value).join(APP_DIR));
        }
    }
    // Windows GUI processes have no HOME: keep the DB and the multi-GB
    // artifact cache in the non-roaming %LOCALAPPDATA%.
    #[cfg(windows)]
    if let Some(value) = std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("APPDATA")) {
        if !value.is_empty() {
            return Ok(PathBuf::from(value).join(APP_DIR));
        }
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())
        .ok_or(StoreError::NoDataDir)?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join(APP_DIR))
}

pub fn default_db_path() -> Result<PathBuf, StoreError> {
    Ok(data_root()?.join(DB_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ArtifactKey {
        ArtifactKey {
            source_sha256: "a".repeat(64),
            trim_in_ms: 1_000,
            trim_out_ms: 9_000,
            target_profile: "avchd:1080p".to_string(),
            target_bitrate: 12_000,
            audio_params_hash: "b".repeat(64),
        }
    }

    #[test]
    fn sqlite_cache_round_trips_and_rejects_missing_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Store::open(temp.path().join("dvdforge.db")).expect("store");
        let artifact_path = store.artifact_blob_path(&key(), "mkv");
        std::fs::create_dir_all(artifact_path.parent().unwrap()).expect("cache dir");
        std::fs::write(&artifact_path, b"cached").expect("artifact");
        store
            .put_artifact(&key(), &artifact_path, 8_000)
            .expect("insert");
        let cached = store.lookup_artifact(&key()).expect("lookup").expect("hit");
        assert_eq!(cached.duration_ms, 8_000);
        assert_eq!(cached.artifact_path, artifact_path.display().to_string());

        std::fs::remove_file(&artifact_path).expect("remove");
        assert!(store.lookup_artifact(&key()).expect("lookup").is_none());
    }

    #[test]
    fn recent_projects_are_upserted_and_sorted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Store::open(temp.path().join("dvdforge.db")).expect("store");
        store
            .record_recent(&RecentProjectInput {
                file_path: "/tmp/one.json".to_string(),
                name: "Uno".to_string(),
                disc_kind: "DVD-9".to_string(),
                thumbnail_path: None,
            })
            .expect("recent");
        store
            .record_recent(&RecentProjectInput {
                file_path: "/tmp/one.json".to_string(),
                name: "Uno actualizado".to_string(),
                disc_kind: "BD-25".to_string(),
                thumbnail_path: Some("/tmp/thumb.png".to_string()),
            })
            .expect("recent");
        let recent = store.recent_projects(10).expect("list");
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].name, "Uno actualizado");
        assert_eq!(recent[0].disc_kind, "BD-25");
    }

    #[test]
    fn job_history_records_status_and_summary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Store::open(temp.path().join("dvdforge.db")).expect("store");
        let job = NewJobHistory {
            id: "job-1".to_string(),
            project_name: "Proyecto".to_string(),
            disc_kind: "DVD-5".to_string(),
            iso_path: Some("/tmp/disc.iso".to_string()),
            iso_size_bytes: 42,
            duration_seconds: 12.5,
            status: "completed".to_string(),
            log_summary: "ok".to_string(),
        };
        store.record_job(&job).expect("job");
        let jobs = store.job_history(10).expect("history");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].status, "completed");
        assert_eq!(jobs[0].duration_seconds, 12.5);
    }
}
