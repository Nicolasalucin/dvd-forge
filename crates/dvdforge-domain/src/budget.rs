use crate::diagnostic::Diagnostic;
use crate::geometry::FrameRate;
use crate::ids::EpisodeId;
use crate::project::{
    AuthoringMode, AvchdResolution, DiscCapacity, DvdProfile, EncodingPriority, H264Level,
    LayerBreakMode,
};
use serde::{Deserialize, Serialize};

/// Capacity policy shared by the planner and the UI.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BudgetPolicy {
    /// 990_000 = 99 % of the effective VOB budget.
    pub target_fill_ppm: u32,
    /// UDF/IFO/VMG/VTS/padding reserve. 30_000 = 3 %.
    pub header_margin_ppm: u32,
    /// MPEG-PS + NAV estimate applied to elementary streams. 25_000 = 2.5 %.
    pub mux_overhead_ppm: u32,
    /// NAV overhead applied when measuring an already muxed `.mpg`. 10_000 = 1 %.
    pub nav_overhead_ppm: u32,
    pub fixed_reserve_bytes: u64,
    pub min_video_kbps: u32,
    pub max_video_kbps: u32,
    pub default_ac3_kbps: u16,
}

impl Default for BudgetPolicy {
    fn default() -> Self {
        Self {
            target_fill_ppm: 990_000,
            header_margin_ppm: 30_000,
            mux_overhead_ppm: 25_000,
            nav_overhead_ppm: 10_000,
            fixed_reserve_bytes: 0,
            min_video_kbps: 2_500,
            max_video_kbps: 9_800,
            default_ac3_kbps: 192,
        }
    }
}

impl BudgetPolicy {
    /// Maximum bytes available for VOBs after the header/filesystem reserve and
    /// the fill target. This is the number the planner must not exceed.
    pub fn effective_vob_bytes(&self, capacity: DiscCapacity) -> u64 {
        let nominal = capacity.nominal_bytes() as u128;
        let after_header = nominal * (1_000_000 - self.header_margin_ppm as u128) / 1_000_000;
        let target = after_header * self.target_fill_ppm as u128 / 1_000_000;
        target.saturating_sub(self.fixed_reserve_bytes as u128) as u64
    }

    pub fn header_margin_bytes(&self, capacity: DiscCapacity) -> u64 {
        (capacity.nominal_bytes() as u128 * self.header_margin_ppm as u128 / 1_000_000) as u64
    }

    pub fn mux_factor(&self) -> f64 {
        1.0 + self.mux_overhead_ppm as f64 / 1_000_000.0
    }

    pub fn nav_factor(&self) -> f64 {
        1.0 + self.nav_overhead_ppm as f64 / 1_000_000.0
    }

    pub fn validate(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        if self.target_fill_ppm > 1_000_000 {
            diagnostics.push(Diagnostic::error(
                "E_TARGET_FILL",
                "El objetivo de llenado no puede superar 1.000.000 ppm.",
            ));
        }
        if self.header_margin_ppm > 1_000_000 || self.mux_overhead_ppm > 1_000_000 {
            diagnostics.push(Diagnostic::error(
                "E_OVERHEAD_RANGE",
                "Los márgenes de cabecera y multiplexación deben estar entre 0 y 1.000.000 ppm.",
            ));
        }
        if self.header_margin_ppm as u64 + self.mux_overhead_ppm as u64 >= 1_000_000 {
            diagnostics.push(Diagnostic::error(
                "E_OVERHEAD_SUM",
                "La suma de márgenes deja el presupuesto de video en cero.",
            ));
        }
        if self.min_video_kbps == 0 || self.min_video_kbps > self.max_video_kbps {
            diagnostics.push(Diagnostic::error(
                "E_VIDEO_KBPS_RANGE",
                "El rango de bitrate de video no es válido.",
            ));
        }
        if !matches!(self.default_ac3_kbps, 192 | 224 | 256 | 320 | 384 | 448) {
            diagnostics.push(Diagnostic::warning(
                "W_AC3_KBPS",
                "El bitrate AC-3 por defecto no es uno de los valores habituales de DVD.",
            ));
        }
        diagnostics
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AvchdPlanDetails {
    pub resolution: AvchdResolution,
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub interlaced: bool,
    pub level: H264Level,
    pub ts_overhead_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BitratePlan {
    pub mode: AuthoringMode,
    pub capacity_bytes: u64,
    pub header_margin_bytes: u64,
    pub target_bytes: u64,
    pub intro_bytes: u64,
    pub menus_bytes: u64,
    pub episode_audio_bytes: u64,
    pub subtitle_bytes: u64,
    pub ifo_bytes: u64,
    pub video_budget_bytes: u64,
    pub total_duration_ms: u64,
    pub video_kbps: u32,
    /// Perfil DVD; en AVCHD se mantiene como valor informativo heredado.
    pub profile: DvdProfile,
    pub avchd: Option<AvchdPlanDetails>,
    /// Sector ECC-aligned donde debería caer el cambio de capa en DVD-9.
    pub layer_break_sector: Option<u64>,
    pub layer_break_mode: Option<LayerBreakMode>,
    pub per_episode: Vec<EpisodeBitrate>,
    pub fill_ppm: u32,
    pub feasible: bool,
    pub fallback: Option<PlanFallback>,
    pub warnings: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeBitrate {
    pub episode: EpisodeId,
    pub duration_ms: u64,
    pub video_kbps: u32,
    pub audio_kbps: u32,
    pub priority: EncodingPriority,
    pub estimated_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlanFallback {
    SwitchToHalfD1 { estimated_kbps: u32 },
    LowerAudioBitrate { from_kbps: u16, to_kbps: u16 },
    ReduceFillTarget { new_fill_ppm: u32 },
    SplitAcrossDiscs { suggested_discs: u8 },
    SwitchToAvchdSd { estimated_kbps: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dvd5_effective_budget_matches_plan() {
        let policy = BudgetPolicy::default();
        assert_eq!(
            policy.effective_vob_bytes(DiscCapacity::Dvd5),
            4_513_410_000
        );
        assert_eq!(policy.header_margin_bytes(DiscCapacity::Dvd5), 141_000_000);
    }

    #[test]
    fn blu_ray_capacities_expose_nominal_and_effective_bytes() {
        assert_eq!(DiscCapacity::Bd25.nominal_bytes(), 25_000_000_000);
        assert_eq!(DiscCapacity::Bd50.nominal_bytes(), 50_000_000_000);
        assert_eq!(DiscCapacity::Bd25.effective_bytes(), 24_996_709_662);
        assert_eq!(DiscCapacity::Bd50.effective_bytes(), 49_993_419_325);
        assert!(DiscCapacity::Bd25.is_blu_ray());
        assert_eq!(DiscCapacity::Bd25.max_video_kbps(), 38_000);
        assert_eq!(DiscCapacity::Dvd9.max_video_kbps(), 9_800);
    }

    #[test]
    fn default_policy_is_valid() {
        let diagnostics = BudgetPolicy::default().validate();
        assert!(diagnostics.iter().all(|d| !d.is_error()));
    }

    #[test]
    fn rejects_impossible_overhead() {
        let policy = BudgetPolicy {
            header_margin_ppm: 600_000,
            mux_overhead_ppm: 500_000,
            ..BudgetPolicy::default()
        };
        let diagnostics = policy.validate();
        assert!(diagnostics.iter().any(|d| d.code == "E_OVERHEAD_SUM"));
    }
}
