//! SQLite persistence for recent projects, the transcoding cache and job
//! history. The default database lives in
//! `~/.local/share/org.dvdforge.desktop/dvdforge.db`.

use chrono::Utc;
use rusqlite::{params, Connection};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactKind {
    #[default]
    Episode,
    MenuVideo,
    MenuOverlay,
    Intro,
}

impl ArtifactKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Episode => "episode",
            Self::MenuVideo => "menuVideo",
            Self::MenuOverlay => "menuOverlay",
            Self::Intro => "intro",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactKey {
    pub kind: ArtifactKind,
    pub source_fingerprint: String,
    pub inputs_hash: String,
    pub target_profile: String,
    pub target_bitrate: u32,
    pub audio_params_hash: String,
    pub encoder_rev: u32,
    pub toolchain_hash: String,
}

impl ArtifactKey {
    pub fn cache_id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.kind.as_str().as_bytes());
        hasher.update(self.source_fingerprint.as_bytes());
        hasher.update(self.inputs_hash.as_bytes());
        hasher.update(self.target_profile.as_bytes());
        hasher.update(self.target_bitrate.to_be_bytes());
        hasher.update(self.audio_params_hash.as_bytes());
        hasher.update(self.encoder_rev.to_be_bytes());
        hasher.update(self.toolchain_hash.as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CachedArtifact {
    pub artifact_path: String,
    pub duration_ms: u64,
    pub size_bytes: u64,
    pub created_at: String,
    pub last_used_at: String,
}

#[derive(Debug)]
pub enum CacheLookup {
    Hit(CachedArtifact),
    Miss,
    Stale(String),
    Error(StoreError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub entries: u64,
    pub bytes: u64,
    pub oldest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PruneReport {
    pub removed_entries: u64,
    pub removed_bytes: u64,
    pub kept_entries: u64,
    pub kept_bytes: u64,
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
            PRAGMA user_version = 2;
            CREATE TABLE IF NOT EXISTS recent_projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                disc_kind TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                thumbnail_path TEXT
            );
            CREATE TABLE IF NOT EXISTS artifact_cache_v2 (
                cache_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                source_fingerprint TEXT NOT NULL,
                inputs_hash TEXT NOT NULL,
                target_profile TEXT NOT NULL,
                target_bitrate INTEGER NOT NULL,
                audio_params_hash TEXT NOT NULL,
                encoder_rev INTEGER NOT NULL,
                toolchain_hash TEXT NOT NULL,
                artifact_path TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                last_used_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS artifact_cache_v2_used
                ON artifact_cache_v2(last_used_at);
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
        // Preserve old v1 rows for visibility. They intentionally will not be
        // hit by v2 keys because v1 did not record the canonical input hash.
        let has_v1: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='artifact_cache')",
            [],
            |row| row.get(0),
        )?;
        if has_v1 {
            let mut statement = connection.prepare(
                "SELECT source_sha256, trim_in_ms, trim_out_ms, target_profile,
                        target_bitrate, audio_params_hash, artifact_path, duration_ms, created_at
                 FROM artifact_cache",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })?;
            for row in rows {
                let (
                    source,
                    trim_in,
                    trim_out,
                    profile,
                    bitrate,
                    audio,
                    artifact_path,
                    duration,
                    created_at,
                ) = row?;
                let key = ArtifactKey {
                    kind: ArtifactKind::Episode,
                    source_fingerprint: source,
                    inputs_hash: format!("legacy-v1:{}:{}", trim_in, trim_out),
                    target_profile: profile,
                    target_bitrate: bitrate.max(0) as u32,
                    audio_params_hash: audio,
                    encoder_rev: 0,
                    toolchain_hash: "legacy-v1".to_string(),
                };
                let _ = connection.execute(
                    "INSERT OR IGNORE INTO artifact_cache_v2 (
                        cache_id, kind, source_fingerprint, inputs_hash, target_profile,
                        target_bitrate, audio_params_hash, encoder_rev, toolchain_hash,
                        artifact_path, duration_ms, size_bytes, created_at, last_used_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        key.cache_id(),
                        key.kind.as_str(),
                        key.source_fingerprint,
                        key.inputs_hash,
                        key.target_profile,
                        key.target_bitrate as i64,
                        key.audio_params_hash,
                        key.encoder_rev as i64,
                        key.toolchain_hash,
                        artifact_path,
                        duration,
                        std::fs::metadata(&artifact_path)
                            .map(|metadata| metadata.len() as i64)
                            .unwrap_or(0),
                        created_at,
                        created_at,
                    ],
                );
            }
        }
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

    pub fn lookup_artifact(&self, key: &ArtifactKey) -> CacheLookup {
        let connection = match self.connection() {
            Ok(connection) => connection,
            Err(error) => return CacheLookup::Error(error),
        };
        let row = connection.query_row(
            "SELECT artifact_path, duration_ms, size_bytes, created_at, last_used_at
             FROM artifact_cache_v2 WHERE cache_id = ?1",
            params![key.cache_id()],
            |row| {
                Ok(CachedArtifact {
                    artifact_path: row.get(0)?,
                    duration_ms: row.get::<_, i64>(1)?.max(0) as u64,
                    size_bytes: row.get::<_, i64>(2)?.max(0) as u64,
                    created_at: row.get(3)?,
                    last_used_at: row.get(4)?,
                })
            },
        );
        match row {
            Ok(artifact) => {
                if !Path::new(&artifact.artifact_path).is_file() {
                    let _ = self.remove_artifact(key);
                    return CacheLookup::Stale(format!(
                        "el blob {} ya no existe",
                        artifact.artifact_path
                    ));
                }
                let now = Utc::now().to_rfc3339();
                let _ = connection.execute(
                    "UPDATE artifact_cache_v2 SET last_used_at = ?1 WHERE cache_id = ?2",
                    params![now, key.cache_id()],
                );
                CacheLookup::Hit(artifact)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => CacheLookup::Miss,
            Err(error) => CacheLookup::Error(error.into()),
        }
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
        let size_bytes = std::fs::metadata(artifact_path)
            .map(|meta| meta.len())
            .unwrap_or(0);
        connection.execute(
            "INSERT INTO artifact_cache_v2 (
                cache_id, kind, source_fingerprint, inputs_hash, target_profile,
                target_bitrate, audio_params_hash, encoder_rev, toolchain_hash,
                artifact_path, duration_ms, size_bytes, created_at, last_used_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(cache_id) DO UPDATE SET
                artifact_path = excluded.artifact_path,
                duration_ms = excluded.duration_ms,
                size_bytes = excluded.size_bytes,
                last_used_at = excluded.last_used_at",
            params![
                key.cache_id(),
                key.kind.as_str(),
                key.source_fingerprint,
                key.inputs_hash,
                key.target_profile,
                key.target_bitrate as i64,
                key.audio_params_hash,
                key.encoder_rev as i64,
                key.toolchain_hash,
                path,
                duration_ms as i64,
                size_bytes as i64,
                now,
                now,
            ],
        )?;
        Ok(CachedArtifact {
            artifact_path: path,
            duration_ms,
            size_bytes,
            created_at: now.clone(),
            last_used_at: now,
        })
    }

    pub fn remove_artifact(&self, key: &ArtifactKey) -> Result<(), StoreError> {
        let connection = self.connection()?;
        connection.execute(
            "DELETE FROM artifact_cache_v2 WHERE cache_id = ?1",
            params![key.cache_id()],
        )?;
        Ok(())
    }

    pub fn cache_stats(&self) -> Result<CacheStats, StoreError> {
        let connection = self.connection()?;
        let (entries, bytes, oldest): (i64, i64, Option<String>) = connection.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0), MIN(last_used_at)
             FROM artifact_cache_v2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok(CacheStats {
            entries: entries.max(0) as u64,
            bytes: bytes.max(0) as u64,
            oldest,
        })
    }

    pub fn prune_artifacts(&self, max_bytes: u64) -> Result<PruneReport, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT cache_id, artifact_path, size_bytes FROM artifact_cache_v2
             ORDER BY last_used_at ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
            ))
        })?;
        let mut current = 0u64;
        let mut pending = Vec::new();
        for row in rows {
            let (cache_id, path, size) = row?;
            current = current.saturating_add(size);
            pending.push((cache_id, path, size));
        }
        let mut removed_entries = 0u64;
        let mut removed_bytes = 0u64;
        let mut kept_entries = 0u64;
        let mut kept_bytes = 0u64;
        for (cache_id, path, size) in pending {
            if current.saturating_sub(removed_bytes) > max_bytes {
                let _ = std::fs::remove_file(&path);
                connection.execute(
                    "DELETE FROM artifact_cache_v2 WHERE cache_id = ?1",
                    params![cache_id],
                )?;
                removed_entries += 1;
                removed_bytes = removed_bytes.saturating_add(size);
            } else {
                kept_entries += 1;
                kept_bytes = kept_bytes.saturating_add(size);
            }
        }
        Ok(PruneReport {
            removed_entries,
            removed_bytes,
            kept_entries,
            kept_bytes,
        })
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
            kind: ArtifactKind::Episode,
            source_fingerprint: "fp2:test".to_string(),
            inputs_hash: "inputs".to_string(),
            target_profile: "avchd:1080p".to_string(),
            target_bitrate: 12_000,
            audio_params_hash: "b".repeat(64),
            encoder_rev: 2,
            toolchain_hash: "ffmpeg-test".to_string(),
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
        let cached = match store.lookup_artifact(&key()) {
            CacheLookup::Hit(artifact) => artifact,
            other => panic!("expected cache hit, got {other:?}"),
        };
        assert_eq!(cached.duration_ms, 8_000);
        assert_eq!(cached.artifact_path, artifact_path.display().to_string());

        std::fs::remove_file(&artifact_path).expect("remove");
        assert!(matches!(
            store.lookup_artifact(&key()),
            CacheLookup::Stale(_)
        ));
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
    fn artifact_cache_key_changes_for_every_input_field() {
        let base = key();
        let variants = [
            ArtifactKey {
                kind: ArtifactKind::MenuVideo,
                ..base.clone()
            },
            ArtifactKey {
                source_fingerprint: "fp2:other".into(),
                ..base.clone()
            },
            ArtifactKey {
                inputs_hash: "other".into(),
                ..base.clone()
            },
            ArtifactKey {
                target_profile: "dvd:other".into(),
                ..base.clone()
            },
            ArtifactKey {
                target_bitrate: 4_000,
                ..base.clone()
            },
            ArtifactKey {
                audio_params_hash: "other".into(),
                ..base.clone()
            },
            ArtifactKey {
                encoder_rev: 3,
                ..base.clone()
            },
            ArtifactKey {
                toolchain_hash: "other".into(),
                ..base.clone()
            },
        ];
        for (index, variant) in variants.iter().enumerate() {
            assert_ne!(base.cache_id(), variant.cache_id(), "variant {index}");
        }
    }

    #[test]
    fn prune_artifacts_removes_oldest_blobs_until_under_budget() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Store::open(temp.path().join("dvdforge.db")).expect("store");
        for index in 0..3 {
            let key = ArtifactKey {
                kind: ArtifactKind::Episode,
                source_fingerprint: format!("fp2:{index}"),
                inputs_hash: format!("inputs-{index}"),
                ..key()
            };
            let path = store.artifact_blob_path(&key, "mpg");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, vec![0u8; 1024]).unwrap();
            store.put_artifact(&key, &path, 1000).unwrap();
        }
        let report = store.prune_artifacts(1024).expect("prune");
        assert_eq!(report.removed_entries, 2);
        assert_eq!(report.kept_entries, 1);
        assert_eq!(report.kept_bytes, 1024);
        assert_eq!(store.cache_stats().unwrap().entries, 1);
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
