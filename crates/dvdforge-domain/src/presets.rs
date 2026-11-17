//! Player compatibility presets.
//!
//! A preset records which player the disc is made for. `apply` tightens the
//! encoder limits for that player and `check` reports anything in the project
//! the player is known to struggle with. Limits are conservative values that
//! are widely reported to play smoothly; they never relax the format limits.

use crate::project::{AuthoringMode, H264Level, Project, VideoConfig};
use crate::Diagnostic;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PlayerPreset {
    /// No extra restriction (format limits only).
    #[default]
    Any,
    /// PlayStation 2 (DVD-Video only).
    Ps2,
    /// PlayStation 3 / 4 / 5.
    PlayStation,
    /// Living-room Blu-ray player (Sony, Panasonic, LG, Samsung…).
    BluRayPlayer,
    /// Older living-room DVD player.
    DvdPlayer,
}

/// DVD-Video peak video bitrate (kbps) that older drives read smoothly.
pub const PS2_MAX_VIDEO_KBPS: u32 = 8_000;
pub const DVD_PLAYER_MAX_VIDEO_KBPS: u32 = 8_500;
/// Blu-ray peak bitrate most standalone players handle without stutter.
pub const BLURAY_PLAYER_MAX_KBPS: u32 = 35_000;

impl PlayerPreset {
    pub const ALL: [PlayerPreset; 5] = [
        PlayerPreset::Any,
        PlayerPreset::Ps2,
        PlayerPreset::PlayStation,
        PlayerPreset::BluRayPlayer,
        PlayerPreset::DvdPlayer,
    ];

    /// Adjusts encoder limits for the player. Returns a description of each
    /// change (Spanish, shown in the UI log).
    pub fn apply(self, project: &mut Project) -> Vec<String> {
        let mut changes = Vec::new();
        match (&mut project.disc.video, self) {
            (VideoConfig::Dvd(config), PlayerPreset::Ps2 | PlayerPreset::DvdPlayer) => {
                let cap = if self == PlayerPreset::Ps2 {
                    PS2_MAX_VIDEO_KBPS
                } else {
                    DVD_PLAYER_MAX_VIDEO_KBPS
                };
                if config.max_video_kbps > cap {
                    changes.push(format!(
                        "Bitrate máximo de vídeo: {} → {cap} kbps.",
                        config.max_video_kbps
                    ));
                    config.max_video_kbps = cap;
                }
                if config.min_video_kbps > cap {
                    config.min_video_kbps = cap;
                }
            }
            (
                VideoConfig::Avchd(config),
                PlayerPreset::BluRayPlayer | PlayerPreset::PlayStation,
            ) => {
                if config.vbv_maxrate_kbps > BLURAY_PLAYER_MAX_KBPS {
                    changes.push(format!(
                        "Bitrate máximo: {} → {BLURAY_PLAYER_MAX_KBPS} kbps.",
                        config.vbv_maxrate_kbps
                    ));
                    config.vbv_maxrate_kbps = BLURAY_PLAYER_MAX_KBPS;
                }
                if !config.is_high_frame_rate_1080p() && config.level != H264Level::Level41 {
                    changes.push("Nivel H.264: 4.1 (el más compatible).".to_string());
                    config.level = H264Level::Level41;
                }
            }
            _ => {}
        }
        project.disc.player_preset = self;
        changes
    }

    /// Diagnostics for what the selected player may not play well.
    pub fn check(self, project: &Project) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let conflict = |message: &str| Diagnostic::error("E_PLAYER_PRESET_CONFLICT", message);
        let strict = |message: String| Diagnostic::warning("W_PLAYER_PRESET_STRICT", message);
        match (self, &project.disc.video) {
            (PlayerPreset::Ps2 | PlayerPreset::DvdPlayer, VideoConfig::Avchd(_))
                if project.disc.mode == AuthoringMode::Avchd =>
            {
                diagnostics.push(conflict(
                    "Este reproductor solo lee DVD-Video: crea un proyecto DVD-Video para él.",
                ));
            }
            (PlayerPreset::Ps2, VideoConfig::Dvd(config))
                if config.max_video_kbps > PS2_MAX_VIDEO_KBPS =>
            {
                diagnostics.push(strict(format!(
                    "La PS2 puede saltar con más de {PS2_MAX_VIDEO_KBPS} kbps de vídeo; el proyecto permite {}.",
                    config.max_video_kbps
                )));
            }
            (PlayerPreset::DvdPlayer, VideoConfig::Dvd(config))
                if config.max_video_kbps > DVD_PLAYER_MAX_VIDEO_KBPS =>
            {
                diagnostics.push(strict(format!(
                    "Algunos reproductores de DVD antiguos saltan con más de {DVD_PLAYER_MAX_VIDEO_KBPS} kbps; el proyecto permite {}.",
                    config.max_video_kbps
                )));
            }
            (
                PlayerPreset::BluRayPlayer | PlayerPreset::PlayStation,
                VideoConfig::Avchd(config),
            ) => {
                if config.is_high_frame_rate_1080p() {
                    diagnostics.push(strict(
                        "1080p a 50/59,94 fps no se reproduce en la PS3 ni en muchos reproductores Blu-ray de salón; usa 25/29,97 fps para máxima compatibilidad."
                            .to_string(),
                    ));
                }
                if config.vbv_maxrate_kbps > BLURAY_PLAYER_MAX_KBPS {
                    diagnostics.push(strict(format!(
                        "Con más de {BLURAY_PLAYER_MAX_KBPS} kbps algunos reproductores pueden saltar; el proyecto permite {}.",
                        config.vbv_maxrate_kbps
                    )));
                }
            }
            _ => {}
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{AvchdResolution, AvchdVideoConfig, DvdVideoConfig};
    use crate::FrameRate;

    fn project_with(video: VideoConfig, mode: AuthoringMode) -> Project {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/projects/minimal.json");
        let mut project: Project =
            serde_json::from_slice(&std::fs::read(path).expect("fixture")).expect("project");
        project.disc.mode = mode;
        project.disc.video = video;
        project
    }

    #[test]
    fn ps2_caps_dvd_bitrate_and_rejects_hd() {
        let mut project = project_with(
            VideoConfig::Dvd(DvdVideoConfig {
                max_video_kbps: 9_800,
                ..serde_json::from_value(serde_json::json!({
                    "profile": "auto", "autoHalfD1AfterMs": 9000000, "maxVideoKbps": 9800,
                    "minVideoKbps": 2500, "gopFrames": 12, "bFrames": 2, "twoPass": true, "trellis": true
                }))
                .unwrap()
            }),
            AuthoringMode::DvdVideo,
        );
        assert!(!PlayerPreset::Ps2.check(&project).is_empty());
        let changes = PlayerPreset::Ps2.apply(&mut project);
        assert_eq!(changes.len(), 1);
        assert!(PlayerPreset::Ps2.check(&project).is_empty());
        assert_eq!(project.disc.player_preset, PlayerPreset::Ps2);

        let hd = project_with(
            VideoConfig::Avchd(AvchdVideoConfig::default()),
            AuthoringMode::Avchd,
        );
        assert!(PlayerPreset::Ps2
            .check(&hd)
            .iter()
            .any(|diagnostic| diagnostic.code == "E_PLAYER_PRESET_CONFLICT"));
    }

    #[test]
    fn bluray_player_warns_on_high_frame_rate_and_caps_bitrate() {
        let mut project = project_with(
            VideoConfig::Avchd(AvchdVideoConfig {
                resolution: AvchdResolution::R1080p,
                frame_rate: FrameRate::FPS_59_94,
                vbv_maxrate_kbps: 38_000,
                ..AvchdVideoConfig::default()
            }),
            AuthoringMode::Avchd,
        );
        let diagnostics = PlayerPreset::BluRayPlayer.check(&project);
        assert_eq!(diagnostics.len(), 2);
        PlayerPreset::BluRayPlayer.apply(&mut project);
        let VideoConfig::Avchd(config) = &project.disc.video else {
            unreachable!()
        };
        assert_eq!(config.vbv_maxrate_kbps, BLURAY_PLAYER_MAX_KBPS);
        // HFR remains a (documented) warning; the user chose that frame rate.
        assert_eq!(PlayerPreset::BluRayPlayer.check(&project).len(), 1);
        assert!(PlayerPreset::Any.check(&project).is_empty());
    }
}
