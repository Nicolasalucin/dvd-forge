//! Bitrate planning for DVD-Video.
//!
//! The calculator follows the approved Section 8 model:
//! nominal capacity -> header reserve -> fill target -> fixed menu/intro
//! reserve -> MPEG-PS/NAV overhead -> episode audio/subtitles -> video bitrate.

use dvdforge_domain::{
    AssetId, AuthoringMode, AvchdPlanDetails, AvchdVideoConfig, Diagnostic, DiscCapacity,
    DvdProfile, EncodingPriority, Episode, EpisodeId, FrameRate, LayerBreakMode, MediaAsset,
    MediaProbe, Project, VideoConfig,
};
use std::collections::HashMap;

pub use dvdforge_domain::budget::{BitratePlan, BudgetPolicy, EpisodeBitrate, PlanFallback};
pub use dvdforge_domain::TvStandard;

pub const CRATE_NAME: &str = "dvdforge-plan";

const MENU_VIDEO_KBPS: u32 = 5_000;
const INTRO_VIDEO_KBPS: u32 = 5_000;
const SUBTITLE_KBPS_PER_TRACK: u32 = 20;
const MENU_OVERHEAD_BYTES: u64 = 2_000_000;
const INTRO_OVERHEAD_BYTES: u64 = 1_000_000;
const IFO_BYTES: u64 = 2 * 1024 * 1024;
const D1_MIN_KBPS: u32 = 2_500;
const HALF_D1_MIN_KBPS: u32 = 1_200;
const DVD_MAX_TOTAL_KBPS: u32 = 10_080;

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("el proyecto tiene errores: {0}")]
    InvalidProject(String),
    #[error("se esperaba un proyecto DVD-Video, no AVCHD")]
    UnsupportedMode,
    #[error("el proyecto no tiene episodios")]
    NoEpisodes,
    #[error("el asset {asset} no existe")]
    MissingAsset { asset: AssetId },
    #[error("el episodio {episode} no tiene una duración conocida")]
    MissingEpisodeDuration { episode: EpisodeId },
    #[error("no queda presupuesto para video después de menús, intro y audio")]
    NoVideoBudget,
}

#[derive(Debug, Clone, Copy)]
struct EpisodeInput {
    id: EpisodeId,
    number: u16,
    duration_ms: u64,
    audio_kbps: u32,
    subtitle_kbps: u32,
    priority: EncodingPriority,
}

/// Los sectores DVD son de 2048 bytes y el cambio de capa debe caer en un
/// bloque ECC de 16 sectores, tal como exige el estándar DVD-9.
const DVD_SECTOR_BYTES: u64 = 2048;
const DVD_ECC_SECTORS: u64 = 16;

/// Largest layer 0 accepted by both DVD-R DL (2,084,960) and DVD+R DL media.
const DVD9_MAX_LAYER0_SECTORS: u64 = 2_084_960;

/// OTP images are split at the middle of the recorded data (L0 >= L1), rounded
/// up to an ECC block and capped at the layer-0 capacity.
fn dvd9_layer_break_sector(capacity: DiscCapacity, estimated_bytes: u64) -> Option<u64> {
    if capacity != DiscCapacity::Dvd9 {
        return None;
    }
    let sectors = estimated_bytes.div_ceil(DVD_SECTOR_BYTES);
    let middle = sectors.div_ceil(2).div_ceil(DVD_ECC_SECTORS) * DVD_ECC_SECTORS;
    Some(middle.clamp(DVD_ECC_SECTORS, DVD9_MAX_LAYER0_SECTORS))
}

fn layer_break_mode(capacity: DiscCapacity, mode: LayerBreakMode) -> Option<LayerBreakMode> {
    (capacity == DiscCapacity::Dvd9).then_some(mode)
}

/// Computes a deterministic DVD-Video bitrate plan from the project and cached
/// or freshly probed media metadata.
pub fn calculate_bitrate_plan(
    project: &Project,
    probes: &HashMap<AssetId, MediaProbe>,
) -> Result<BitratePlan, PlanError> {
    let errors: Vec<String> = project
        .validate()
        .into_iter()
        .filter(Diagnostic::is_error)
        .map(|diagnostic| diagnostic.to_string())
        .collect();
    if !errors.is_empty() {
        return Err(PlanError::InvalidProject(errors.join("; ")));
    }

    if let VideoConfig::Avchd(config) = &project.disc.video {
        return calculate_avchd_plan(project, probes, config);
    }
    let (requested_profile, auto_half_d1_after_ms) = match &project.disc.video {
        VideoConfig::Dvd(config) => (config.profile, config.auto_half_d1_after_ms),
        VideoConfig::Avchd(_) => return Err(PlanError::UnsupportedMode),
    };
    if project.episodes.is_empty() {
        return Err(PlanError::NoEpisodes);
    }

    let capacity = project.disc.capacity;
    let policy = &project.budget;
    let vob_budget = policy.effective_vob_bytes(capacity);
    let mut warnings = Vec::new();

    let audio_layout = project.episodes.first().map(|episode| episode.audio.len());
    for episode in &project.episodes {
        if audio_layout.is_some_and(|count| count != episode.audio.len()) {
            warnings.push(Diagnostic::warning(
                "W_AUDIO_LAYOUT_MISMATCH",
                format!(
                    "El episodio {} tiene {} pistas de audio y el primero tiene {}. dvdauthor usa el layout del primer título.",
                    episode.number,
                    episode.audio.len(),
                    audio_layout.unwrap_or(0)
                ),
            ));
        }
        if episode
            .audio
            .iter()
            .any(|audio| audio.source_path.is_some())
        {
            warnings.push(Diagnostic::warning(
                "W_EXTERNAL_AUDIO",
                format!(
                    "El episodio {} referencia audio externo; el encoder usa el stream embebido del vídeo.",
                    episode.number
                ),
            ));
        }
        if episode
            .subtitles
            .iter()
            .any(|subtitle| subtitle.source_path.is_none())
        {
            warnings.push(Diagnostic::warning(
                "W_SUBTITLE_STREAM_SOURCE",
                format!(
                    "El episodio {} tiene subtítulos internos; se extraerán del vídeo con FFmpeg antes de multiplexarlos.",
                    episode.number
                ),
            ));
        }
    }

    let menus_bytes = estimate_menus_bytes(project);
    let intro_bytes = estimate_intro_bytes(project, probes, &mut warnings);
    let ifo_bytes = IFO_BYTES;
    let fixed_bytes = menus_bytes
        .saturating_add(intro_bytes)
        .saturating_add(ifo_bytes);
    if fixed_bytes >= vob_budget {
        return Err(PlanError::NoVideoBudget);
    }

    let episode_vob_budget = vob_budget - fixed_bytes;
    let mux_ppm = policy.mux_overhead_ppm as u64;
    let episode_es_budget = episode_vob_budget.saturating_mul(1_000_000) / (1_000_000 + mux_ppm);

    let mut episodes = Vec::with_capacity(project.episodes.len());
    let mut audio_total = 0u64;
    let mut subtitle_total = 0u64;
    let mut total_duration_ms = 0u64;

    for episode in &project.episodes {
        let asset = project
            .asset(episode.video)
            .ok_or(PlanError::MissingAsset {
                asset: episode.video,
            })?;
        let probe = probe_for(asset, probes);
        let duration_ms = episode_duration_ms(episode, probe, project.disc.standard.frame_rate())
            .ok_or(PlanError::MissingEpisodeDuration {
            episode: episode.id,
        })?;
        if duration_ms == 0 {
            return Err(PlanError::MissingEpisodeDuration {
                episode: episode.id,
            });
        }
        let audio_kbps = episode_audio_kbps(episode, project);
        let subtitle_kbps = episode.subtitles.len() as u32 * SUBTITLE_KBPS_PER_TRACK;
        audio_total = audio_total.saturating_add(bytes_for(duration_ms, audio_kbps));
        subtitle_total = subtitle_total.saturating_add(bytes_for(duration_ms, subtitle_kbps));
        total_duration_ms = total_duration_ms.saturating_add(duration_ms);
        episodes.push(EpisodeInput {
            id: episode.id,
            number: episode.number,
            duration_ms,
            audio_kbps,
            subtitle_kbps,
            priority: episode.encoding_priority,
        });
    }

    if total_duration_ms == 0 {
        return Err(PlanError::NoEpisodes);
    }

    let video_budget_bytes = episode_es_budget
        .saturating_sub(audio_total)
        .saturating_sub(subtitle_total);
    if video_budget_bytes == 0 {
        return Err(PlanError::NoVideoBudget);
    }

    let raw_video_kbps = (video_budget_bytes as u128 * 8 / total_duration_ms as u128) as u64;
    let auto_half_d1 = requested_profile == DvdProfile::Auto
        && (total_duration_ms > auto_half_d1_after_ms || raw_video_kbps < D1_MIN_KBPS as u64);
    let profile = match requested_profile {
        DvdProfile::Auto => {
            if auto_half_d1 {
                DvdProfile::HalfD1
            } else {
                DvdProfile::D1
            }
        }
        explicit => explicit,
    };
    let min_video_kbps = match profile {
        DvdProfile::HalfD1 => HALF_D1_MIN_KBPS,
        DvdProfile::D1 | DvdProfile::Auto => policy.min_video_kbps.max(D1_MIN_KBPS),
    };
    let max_video_kbps = policy.max_video_kbps.max(1);
    let video_kbps = raw_video_kbps.min(max_video_kbps as u64) as u32;

    if auto_half_d1 {
        warnings.push(Diagnostic::warning(
            "W_HALF_D1_AUTO",
            format!(
                "Se activó Half-D1 automáticamente: duración {} ms y bitrate calculado {} kbps.",
                total_duration_ms, raw_video_kbps
            ),
        ));
    }
    if raw_video_kbps > max_video_kbps as u64 {
        warnings.push(Diagnostic::warning(
            "W_BITRATE_CAPPED",
            format!(
                "El bitrate calculado {} kbps supera el máximo {} kbps; el disco quedará por debajo del objetivo de llenado.",
                raw_video_kbps, max_video_kbps
            ),
        ));
    }
    if requested_profile == DvdProfile::D1 && raw_video_kbps < D1_MIN_KBPS as u64 {
        warnings.push(Diagnostic::warning(
            "W_D1_FORCED_LOW",
            format!(
                "D1 está forzado pero el bitrate calculado ({} kbps) es inferior a {} kbps.",
                raw_video_kbps, D1_MIN_KBPS
            ),
        ));
    }

    let allocated = priority_video_kbps(&episodes, video_kbps, max_video_kbps);
    let mut per_episode = Vec::with_capacity(episodes.len());
    let mut estimated_total = fixed_bytes;
    for (episode, allocated_kbps) in episodes.iter().zip(allocated) {
        let max_for_audio = DVD_MAX_TOTAL_KBPS
            .saturating_sub(episode.audio_kbps)
            .saturating_sub(100);
        let episode_video_kbps = allocated_kbps.min(max_for_audio).max(1);
        if episode_video_kbps < allocated_kbps {
            warnings.push(Diagnostic::warning(
                "W_EPISODE_BITRATE_CAP",
                format!(
                    "El episodio {} se limita a {} kbps para respetar el máximo total de DVD con sus pistas de audio.",
                    episode.number, episode_video_kbps
                ),
            ));
        }
        let video_bytes = bytes_for(episode.duration_ms, episode_video_kbps);
        let audio_bytes = bytes_for(episode.duration_ms, episode.audio_kbps);
        let subtitle_bytes = bytes_for(episode.duration_ms, episode.subtitle_kbps);
        let es_bytes = video_bytes
            .saturating_add(audio_bytes)
            .saturating_add(subtitle_bytes);
        let vob_bytes = es_bytes.saturating_mul(1_000_000 + mux_ppm) / 1_000_000;
        estimated_total = estimated_total.saturating_add(vob_bytes);
        per_episode.push(EpisodeBitrate {
            episode: episode.id,
            duration_ms: episode.duration_ms,
            video_kbps: episode_video_kbps,
            audio_kbps: episode.audio_kbps,
            priority: episode.priority,
            estimated_bytes: vob_bytes,
        });
    }

    let effective_capacity = capacity.nominal_bytes() as u128
        * (1_000_000 - policy.header_margin_ppm as u128)
        / 1_000_000;
    let fill_ppm = (estimated_total as u128 * 1_000_000)
        .checked_div(effective_capacity)
        .unwrap_or(0) as u32;

    let under_min = video_kbps < min_video_kbps;
    let overflow = estimated_total > vob_budget;
    let feasible = !under_min && !overflow;
    if feasible && fill_ppm < 950_000 {
        warnings.push(Diagnostic::warning(
            "W_UNDERFILL",
            format!(
                "El plan solo llena el {:.1}% de la capacidad efectiva; el contenido es demasiado corto para alcanzar el 99% sin superar el bitrate máximo.",
                fill_ppm as f64 / 10_000.0
            ),
        ));
    }
    if !feasible {
        warnings.push(Diagnostic::error(
            "E_PLAN_INFEASIBLE",
            "El plan no es viable con el perfil y presupuesto actuales.",
        ));
    }

    let fallback = if !feasible {
        if requested_profile == DvdProfile::D1 && raw_video_kbps < D1_MIN_KBPS as u64 {
            Some(PlanFallback::SwitchToHalfD1 {
                estimated_kbps: u32::try_from(raw_video_kbps).unwrap_or(u32::MAX),
            })
        } else if overflow {
            let divisor = vob_budget.max(1);
            let suggested_discs = estimated_total.div_ceil(divisor).max(2);
            Some(PlanFallback::SplitAcrossDiscs {
                suggested_discs: u8::try_from(suggested_discs).unwrap_or(u8::MAX),
            })
        } else {
            Some(PlanFallback::SplitAcrossDiscs { suggested_discs: 2 })
        }
    } else {
        None
    };

    Ok(BitratePlan {
        mode: AuthoringMode::DvdVideo,
        avchd: None,
        capacity_bytes: capacity.nominal_bytes(),
        header_margin_bytes: policy.header_margin_bytes(capacity),
        target_bytes: vob_budget,
        intro_bytes,
        menus_bytes,
        episode_audio_bytes: audio_total,
        subtitle_bytes: subtitle_total,
        ifo_bytes,
        video_budget_bytes,
        total_duration_ms,
        video_kbps,
        profile,
        layer_break_sector: dvd9_layer_break_sector(capacity, estimated_total),
        layer_break_mode: layer_break_mode(capacity, project.disc.layer_break_mode),
        per_episode,
        fill_ppm,
        feasible,
        fallback,
        warnings,
    })
}

fn calculate_avchd_plan(
    project: &Project,
    probes: &HashMap<AssetId, MediaProbe>,
    config: &AvchdVideoConfig,
) -> Result<BitratePlan, PlanError> {
    let capacity = project.disc.capacity;
    let config_errors: Vec<String> = config
        .validate_for_capacity(capacity)
        .into_iter()
        .filter(Diagnostic::is_error)
        .map(|diagnostic| diagnostic.to_string())
        .collect();
    if !config_errors.is_empty() {
        return Err(PlanError::InvalidProject(config_errors.join("; ")));
    }
    if project.episodes.is_empty() {
        return Err(PlanError::NoEpisodes);
    }

    let nominal = capacity.nominal_bytes();
    let capacity_base = if capacity.is_blu_ray() {
        capacity.effective_bytes()
    } else {
        nominal
    };
    let header_margin = if capacity.is_blu_ray() {
        (capacity_base as u128 * dvdforge_avchd::AVCHD_HEADER_MARGIN_PPM as u128 / 1_000_000) as u64
    } else {
        dvdforge_avchd::header_margin_bytes(capacity)
    };
    let target_bytes = ((capacity_base - header_margin) as u128
        * project.budget.target_fill_ppm as u128
        / 1_000_000) as u64;
    let menus_bytes = project.menus.len() as u64 * 2_000_000;
    let target_bytes = target_bytes.saturating_sub(menus_bytes);
    let es_budget = (target_bytes as u128 * 1_000_000
        / (1_000_000 + dvdforge_avchd::AVCHD_TS_OVERHEAD_PPM as u128)) as u64;
    let ts_overhead_bytes = target_bytes.saturating_sub(es_budget);

    let mut warnings = Vec::new();
    if !project
        .episodes
        .iter()
        .all(|episode| episode.subtitles.is_empty())
    {
        warnings.push(Diagnostic::warning(
            "W_AVCHD_SUBTITLES_IGNORED",
            "Los subtítulos externos de AVCHD/Blu-ray se conservan en el proyecto, pero todavía no se multiplexan en la imagen.",
        ));
    }

    let mut episodes = Vec::with_capacity(project.episodes.len());
    let mut audio_total = 0u64;
    let mut total_duration_ms = 0u64;
    for episode in &project.episodes {
        let asset = project
            .asset(episode.video)
            .ok_or(PlanError::MissingAsset {
                asset: episode.video,
            })?;
        let probe = probe_for(asset, probes);
        let duration_ms = episode_duration_ms(episode, probe, project.disc.standard.frame_rate())
            .ok_or(PlanError::MissingEpisodeDuration {
            episode: episode.id,
        })?;
        if duration_ms == 0 {
            return Err(PlanError::MissingEpisodeDuration {
                episode: episode.id,
            });
        }
        if episode.audio.len() > 1 {
            warnings.push(Diagnostic::warning(
                "W_AVCHD_MULTI_AUDIO",
                format!(
                    "El episodio {} tiene varias pistas de audio; AVCHD usará la primera.",
                    episode.number
                ),
            ));
        }
        let audio_kbps = episode
            .audio
            .first()
            .map(|audio| audio.ac3_kbps as u32)
            .unwrap_or(project.disc.audio.ac3_kbps as u32);
        let audio_bytes = bytes_for(duration_ms, audio_kbps);
        audio_total = audio_total.saturating_add(audio_bytes);
        total_duration_ms = total_duration_ms.saturating_add(duration_ms);
        episodes.push(EpisodeInput {
            id: episode.id,
            number: episode.number,
            duration_ms,
            audio_kbps,
            subtitle_kbps: 0,
            priority: episode.encoding_priority,
        });
    }
    if total_duration_ms == 0 {
        return Err(PlanError::NoEpisodes);
    }

    let video_budget_bytes = es_budget.saturating_sub(audio_total);
    if video_budget_bytes == 0 {
        return Err(PlanError::NoVideoBudget);
    }
    let raw_video_kbps = (video_budget_bytes as u128 * 8 / total_duration_ms as u128) as u64;
    let max_video_cap = if capacity.is_blu_ray() {
        dvdforge_avchd::BLURAY_MAX_VIDEO_KBPS
    } else {
        dvdforge_avchd::AVCHD_MAX_VIDEO_KBPS
    };
    let max_video_kbps = config
        .vbv_maxrate_kbps
        .min(project.budget.max_video_kbps)
        .clamp(1, max_video_cap);
    let video_kbps = raw_video_kbps.min(max_video_kbps as u64).max(1) as u32;
    if raw_video_kbps > max_video_kbps as u64 {
        warnings.push(Diagnostic::warning(
            "W_AVCHD_BITRATE_CAPPED",
            format!(
                "El bitrate calculado {} kbps supera el máximo de {} kbps para {}; el disco quedará por debajo del objetivo de llenado.",
                raw_video_kbps,
                max_video_kbps,
                capacity.label()
            ),
        ));
    }
    if total_duration_ms > 5 * 3_600_000
        && !matches!(
            config.resolution,
            dvdforge_domain::AvchdResolution::Sd480i | dvdforge_domain::AvchdResolution::Sd576i
        )
    {
        warnings.push(Diagnostic::warning(
            "W_AVCHD_SD_RECOMMENDED",
            "Más de 5 horas de contenido: considera resolución SD para conservar calidad y capacidad.",
        ));
    }

    let (width, height) = config.resolution.dimensions();
    let allocated = priority_video_kbps(&episodes, video_kbps, max_video_kbps);
    let mut per_episode = Vec::with_capacity(episodes.len());
    let mut estimated_total = 0u64;
    for (episode, episode_video_kbps) in episodes.iter().zip(allocated) {
        let video_bytes = bytes_for(episode.duration_ms, episode_video_kbps);
        let audio_bytes = bytes_for(episode.duration_ms, episode.audio_kbps);
        let es_bytes = video_bytes.saturating_add(audio_bytes);
        let ts_bytes = dvdforge_avchd::ts_overhead_bytes(es_bytes);
        let estimated_bytes = es_bytes.saturating_add(ts_bytes);
        estimated_total = estimated_total.saturating_add(estimated_bytes);
        per_episode.push(EpisodeBitrate {
            episode: episode.id,
            duration_ms: episode.duration_ms,
            video_kbps: episode_video_kbps,
            audio_kbps: episode.audio_kbps,
            priority: episode.priority,
            estimated_bytes,
        });
    }

    let fill_ppm = (estimated_total as u128 * 1_000_000)
        .checked_div((capacity_base - header_margin) as u128)
        .unwrap_or(0) as u32;
    if fill_ppm < 950_000 {
        warnings.push(Diagnostic::warning(
            "W_AVCHD_UNDERFILL",
            format!(
                "El plan llena el {:.1}% de la capacidad efectiva de {}; el contenido es demasiado corto para aprovecharla.",
                fill_ppm as f64 / 10_000.0,
                capacity.label()
            ),
        ));
    }
    let overflow = estimated_total > target_bytes;
    let feasible = !overflow;
    if !feasible {
        warnings.push(Diagnostic::error(
            "E_AVCHD_PLAN_INFEASIBLE",
            "El plan AVCHD supera el presupuesto del disco.",
        ));
    }
    let fallback = overflow.then(|| PlanFallback::SplitAcrossDiscs {
        suggested_discs: u8::try_from(estimated_total.div_ceil(target_bytes.max(1)))
            .unwrap_or(u8::MAX)
            .max(2),
    });

    Ok(BitratePlan {
        mode: AuthoringMode::Avchd,
        avchd: Some(AvchdPlanDetails {
            resolution: config.resolution,
            width,
            height,
            frame_rate: config.frame_rate,
            interlaced: config.resolution.interlaced(),
            level: config.level,
            ts_overhead_bytes,
        }),
        capacity_bytes: nominal,
        header_margin_bytes: header_margin,
        target_bytes,
        intro_bytes: 0,
        menus_bytes,
        episode_audio_bytes: audio_total,
        subtitle_bytes: 0,
        ifo_bytes: 0,
        video_budget_bytes,
        total_duration_ms,
        video_kbps,
        profile: DvdProfile::D1,
        layer_break_sector: dvd9_layer_break_sector(
            capacity,
            estimated_total.saturating_add(menus_bytes),
        ),
        layer_break_mode: layer_break_mode(capacity, project.disc.layer_break_mode),
        per_episode,
        fill_ppm,
        feasible,
        fallback,
        warnings,
    })
}

fn probe_for<'a>(
    asset: &'a MediaAsset,
    probes: &'a HashMap<AssetId, MediaProbe>,
) -> &'a MediaProbe {
    probes.get(&asset.id).unwrap_or(&asset.probe)
}

fn episode_duration_ms(
    episode: &Episode,
    probe: &MediaProbe,
    fallback_fps: FrameRate,
) -> Option<u64> {
    if episode.trim_end_ms.is_some() {
        let duration = episode.effective_duration_ms();
        if duration > 0 {
            return Some(duration);
        }
    }
    if let Some(start) = episode.trim_start_ms {
        let end = episode.trim_end_ms.or(probe.duration_ms)?;
        let duration = end.saturating_sub(start);
        return (duration > 0).then_some(duration);
    }
    duration_from_probe(probe, episode.trim, fallback_fps)
}

fn duration_from_probe(
    probe: &MediaProbe,
    trim: Option<dvdforge_domain::FrameRange>,
    fallback_fps: FrameRate,
) -> Option<u64> {
    if let Some(trim) = trim {
        let fps = probe
            .video
            .as_ref()
            .map(|video| video.frame_rate)
            .unwrap_or(fallback_fps);
        if !fps.is_valid() || trim.duration() == 0 {
            return None;
        }
        let duration_ms = (trim.duration() as f64 * 1000.0 / fps.as_f64()).round() as u64;
        return (duration_ms > 0).then_some(duration_ms);
    }
    probe.duration_ms.filter(|duration| *duration > 0)
}

fn episode_audio_kbps(episode: &Episode, project: &Project) -> u32 {
    let total: u32 = episode
        .audio
        .iter()
        .map(|audio| audio.ac3_kbps as u32)
        .sum();
    if total == 0 {
        project.disc.audio.ac3_kbps as u32
    } else {
        total
    }
}

fn estimate_menus_bytes(project: &Project) -> u64 {
    project
        .menus
        .iter()
        .map(|menu| {
            let audio_kbps = if menu.audio.is_some() {
                project.disc.audio.ac3_kbps as u32
            } else {
                0
            };
            bytes_for(menu.timing.duration_ms, MENU_VIDEO_KBPS + audio_kbps)
                .saturating_add(MENU_OVERHEAD_BYTES)
        })
        .sum()
}

fn estimate_intro_bytes(
    project: &Project,
    probes: &HashMap<AssetId, MediaProbe>,
    warnings: &mut Vec<Diagnostic>,
) -> u64 {
    let Some(first_play) = &project.first_play else {
        return 0;
    };
    let Some(asset) = project.asset(first_play.asset) else {
        warnings.push(Diagnostic::warning(
            "W_INTRO_ASSET_MISSING",
            "El First Play referencia un asset inexistente; no se reserva espacio para la intro.",
        ));
        return 0;
    };
    let probe = probe_for(asset, probes);
    let Some(duration_ms) =
        duration_from_probe(probe, first_play.trim, project.disc.standard.frame_rate())
    else {
        warnings.push(Diagnostic::warning(
            "W_INTRO_DURATION",
            "No se pudo determinar la duración de la intro; no se reserva espacio.",
        ));
        return 0;
    };
    let audio_kbps = first_play
        .audio
        .as_ref()
        .map(|audio| audio.ac3_kbps as u32)
        .unwrap_or(project.disc.audio.ac3_kbps as u32);
    bytes_for(duration_ms, INTRO_VIDEO_KBPS + audio_kbps).saturating_add(INTRO_OVERHEAD_BYTES)
}

fn bytes_for(duration_ms: u64, kbps: u32) -> u64 {
    (duration_ms as u128 * kbps as u128 / 8) as u64
}

/// Reparte el presupuesto de vídeo manteniendo el promedio global. Una
/// prioridad alta recibe más bits y una baja menos, sin cambiar el total.
fn priority_video_kbps(episodes: &[EpisodeInput], average_kbps: u32, max_kbps: u32) -> Vec<u32> {
    let total_duration: u64 = episodes.iter().map(|episode| episode.duration_ms).sum();
    let weighted_duration: f64 = episodes
        .iter()
        .map(|episode| episode.duration_ms as f64 * episode.priority.weight())
        .sum();
    if total_duration == 0 || weighted_duration <= f64::EPSILON {
        return vec![average_kbps; episodes.len()];
    }
    episodes
        .iter()
        .map(|episode| {
            let share = episode.duration_ms as f64 * episode.priority.weight() / weighted_duration;
            let kbps =
                average_kbps as f64 * share * total_duration as f64 / episode.duration_ms as f64;
            (kbps.round() as u32).clamp(1, max_kbps)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvdforge_project::load_project;
    use std::path::PathBuf;

    fn fixture() -> Project {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/minimal.json");
        load_project(path).expect("fixture must load")
    }

    fn set_duration(project: &mut Project, duration_ms: u64) {
        project.assets[0].probe.duration_ms = Some(duration_ms);
    }

    #[test]
    fn two_hour_dvd5_plan_is_viable() {
        let mut project = fixture();
        set_duration(&mut project, 7_200_000);
        let plan = calculate_bitrate_plan(&project, &HashMap::new()).expect("plan");
        assert!(plan.feasible);
        assert_eq!(plan.profile, DvdProfile::D1);
        assert!(plan.video_kbps > 4_000 && plan.video_kbps < 5_000);
        assert!(plan.fill_ppm >= 980_000 && plan.fill_ppm <= 995_000);
        assert!(!plan.warnings.iter().any(|warning| warning.is_error()));
    }

    #[test]
    fn four_hours_switch_to_half_d1() {
        let mut project = fixture();
        set_duration(&mut project, 14_400_000);
        let plan = calculate_bitrate_plan(&project, &HashMap::new()).expect("plan");
        assert!(plan.feasible);
        assert_eq!(plan.profile, DvdProfile::HalfD1);
        assert!(plan.video_kbps > 1_200 && plan.video_kbps < 2_500);
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "W_HALF_D1_AUTO"));
    }

    #[test]
    fn short_content_caps_bitrate_and_warns_underfill() {
        let mut project = fixture();
        set_duration(&mut project, 120_000);
        let plan = calculate_bitrate_plan(&project, &HashMap::new()).expect("plan");
        assert!(plan.feasible);
        assert_eq!(plan.video_kbps, 9_800);
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "W_BITRATE_CAPPED"));
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "W_UNDERFILL"));
    }

    #[test]
    fn forced_d1_below_minimum_suggests_half_d1() {
        let mut project = fixture();
        set_duration(&mut project, 14_400_000);
        if let VideoConfig::Dvd(config) = &mut project.disc.video {
            config.profile = DvdProfile::D1;
        }
        let plan = calculate_bitrate_plan(&project, &HashMap::new()).expect("plan");
        assert!(!plan.feasible);
        assert!(matches!(
            plan.fallback,
            Some(PlanFallback::SwitchToHalfD1 { .. })
        ));
    }

    #[test]
    fn avchd_fixture_produces_an_avchd_plan() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/projects/avchd-dvd5.json");
        let project = load_project(path).expect("avchd fixture must load");
        let plan = calculate_bitrate_plan(&project, &HashMap::new()).expect("avchd plan");
        assert_eq!(plan.mode, AuthoringMode::Avchd);
        assert!(plan.feasible);
        assert!(plan.video_kbps <= dvdforge_avchd::AVCHD_MAX_VIDEO_KBPS);
        let avchd = plan.avchd.expect("avchd details");
        assert_eq!(avchd.resolution, dvdforge_domain::AvchdResolution::R720p);
        assert_eq!(avchd.width, 1280);
        assert_eq!(avchd.height, 720);
        assert_eq!(avchd.frame_rate, FrameRate::FPS_23_976);
        assert!(!avchd.interlaced);
    }

    #[test]
    fn blu_ray_plan_unlocks_38_mbps() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/projects/avchd-dvd5.json");
        let mut project = load_project(path).expect("avchd fixture must load");
        project.disc.capacity = DiscCapacity::Bd25;
        project.disc.video = VideoConfig::Avchd(AvchdVideoConfig {
            vbv_maxrate_kbps: 38_000,
            vbv_bufsize_kbps: 35_000,
            ..match &project.disc.video {
                VideoConfig::Avchd(config) => config.clone(),
                VideoConfig::Dvd(_) => unreachable!(),
            }
        });
        project.budget.max_video_kbps = 38_000;
        let plan = calculate_bitrate_plan(&project, &HashMap::new()).expect("bd plan");
        assert_eq!(plan.capacity_bytes, DiscCapacity::Bd25.nominal_bytes());
        assert!(plan.video_kbps <= 38_000);
        let avchd = plan.avchd.expect("avchd details");
        assert_eq!(avchd.height, 720);
        assert!(
            plan.feasible
                || plan
                    .warnings
                    .iter()
                    .any(|warning| warning.code == "W_AVCHD_UNDERFILL")
        );
    }

    #[test]
    fn avchd_two_hour_budget_fills_the_dvd5() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/projects/avchd-dvd5.json");
        let mut project = load_project(path).expect("avchd fixture must load");
        for asset in &mut project.assets {
            asset.probe.duration_ms = Some(3_600_000);
        }
        let plan = calculate_bitrate_plan(&project, &HashMap::new()).expect("avchd plan");
        assert!(plan.feasible);
        assert_eq!(plan.mode, AuthoringMode::Avchd);
        assert!(plan.video_kbps > 4_000 && plan.video_kbps < 5_500);
        assert!(plan.fill_ppm > 950_000);
    }

    #[test]
    fn trim_ms_controls_the_planned_duration() {
        let mut project = fixture();
        if let Some(episode) = project.episodes.first_mut() {
            episode.trim_start_ms = Some(60_000);
            episode.trim_end_ms = Some(126_000);
        }
        project.assets[0].probe.duration_ms = Some(7_200_000);
        let plan = calculate_bitrate_plan(&project, &HashMap::new()).expect("plan");
        assert_eq!(plan.total_duration_ms, 66_000);
    }

    #[test]
    fn missing_duration_is_an_error() {
        let mut project = fixture();
        project.assets[0].probe.duration_ms = None;
        let error = calculate_bitrate_plan(&project, &HashMap::new()).expect_err("must fail");
        assert!(matches!(error, PlanError::MissingEpisodeDuration { .. }));
    }

    #[test]
    fn dvd9_layer_break_is_ecc_aligned() {
        assert_eq!(
            dvd9_layer_break_sector(DiscCapacity::Dvd5, 6_000_000_000),
            None
        );
        let sector =
            dvd9_layer_break_sector(DiscCapacity::Dvd9, 6_000_000_000).expect("dvd9 break");
        assert_eq!(sector % DVD_ECC_SECTORS, 0);
        assert!(sector * DVD_SECTOR_BYTES * 2 >= 6_000_000_000);
        assert_eq!(
            dvd9_layer_break_sector(DiscCapacity::Dvd9, 9_000_000_000),
            Some(DVD9_MAX_LAYER0_SECTORS)
        );
        assert_eq!(
            layer_break_mode(DiscCapacity::Dvd9, LayerBreakMode::Ptp),
            Some(LayerBreakMode::Ptp)
        );
    }

    #[test]
    fn priority_weights_redistribute_bitrate_without_changing_average() {
        let episodes = vec![
            EpisodeInput {
                id: EpisodeId::from_u128(1),
                number: 1,
                duration_ms: 60_000,
                audio_kbps: 192,
                subtitle_kbps: 0,
                priority: EncodingPriority::Low,
            },
            EpisodeInput {
                id: EpisodeId::from_u128(2),
                number: 2,
                duration_ms: 60_000,
                audio_kbps: 192,
                subtitle_kbps: 0,
                priority: EncodingPriority::High,
            },
        ];
        let allocated = priority_video_kbps(&episodes, 5_000, 9_800);
        assert!(allocated[1] > allocated[0]);
        let average = allocated.iter().sum::<u32>() / allocated.len() as u32;
        assert!((4_900..=5_100).contains(&average));

        let all_medium = vec![
            EpisodeInput {
                priority: EncodingPriority::Medium,
                ..episodes[0]
            },
            EpisodeInput {
                priority: EncodingPriority::Medium,
                ..episodes[1]
            },
        ];
        assert_eq!(
            priority_video_kbps(&all_medium, 5_000, 9_800),
            vec![5_000, 5_000]
        );
    }

    #[test]
    fn dvd9_plan_marks_the_layer_break() {
        let mut project = fixture();
        project.disc.capacity = DiscCapacity::Dvd9;
        project.disc.layer_break_mode = LayerBreakMode::Ptp;
        set_duration(&mut project, 7_200_000);
        let plan = calculate_bitrate_plan(&project, &HashMap::new()).expect("plan");
        assert!(plan
            .layer_break_sector
            .unwrap()
            .is_multiple_of(DVD_ECC_SECTORS));
        assert_eq!(plan.layer_break_mode, Some(LayerBreakMode::Ptp));
        assert_eq!(plan.capacity_bytes, DiscCapacity::Dvd9.nominal_bytes());
    }

    #[test]
    fn duration_uses_rational_frame_rate_for_trim() {
        let probe = MediaProbe {
            format_name: "test".into(),
            duration_ms: Some(10_000),
            size_bytes: None,
            bitrate_bps: None,
            video: Some(dvdforge_domain::VideoStreamInfo {
                index: 0,
                codec: "h264".into(),
                width: 720,
                height: 480,
                frame_rate: FrameRate {
                    num: 30_000,
                    den: 1_001,
                },
                sample_aspect_ratio: None,
                display_aspect_ratio: None,
                pix_fmt: None,
                field_order: None,
                bit_depth: None,
            }),
            audio: vec![],
            subtitle_streams: 0,
            subtitle_tracks: Vec::new(),
            chapters: Vec::new(),
        };
        let trim = dvdforge_domain::FrameRange {
            start: 0,
            end: 30_000,
        };
        assert_eq!(
            duration_from_probe(&probe, Some(trim), FrameRate::PAL),
            Some(1_001_000)
        );
    }
}
