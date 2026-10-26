//! Job state machine, event payloads and cancellation primitives.

use dvdforge_domain::JobId;
use serde::{Deserialize, Serialize};

pub const CRATE_NAME: &str = "dvdforge-jobs";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum JobStage {
    Probe,
    Plan,
    EncodeIntro,
    EncodeMenus,
    EncodeEpisodes,
    Author,
    Iso,
    Verify,
    Burn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactKind {
    Iso,
    VideoTs,
    Menu,
    Plan,
    Log,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct JobRecord {
    pub id: JobId,
    pub state: JobState,
    pub stage: Option<JobStage>,
    pub permille: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum JobEventKind {
    StageStarted {
        stage: JobStage,
        label: String,
    },
    Progress {
        stage: JobStage,
        permille: u16,
        fps: Option<f64>,
        speed: Option<f64>,
        eta_secs: Option<u64>,
    },
    Warning {
        code: String,
        message: String,
    },
    Log {
        level: LogLevel,
        message: String,
    },
    Artifact {
        kind: ArtifactKind,
        path: String,
    },
    Completed {
        summary: String,
    },
    Failed {
        code: String,
        message: String,
        stage: Option<JobStage>,
        log_path: Option<String>,
    },
    Cancelled {
        stage: Option<JobStage>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub job_id: JobId,
    pub seq: u64,
    pub at: String,
    pub kind: JobEventKind,
}
