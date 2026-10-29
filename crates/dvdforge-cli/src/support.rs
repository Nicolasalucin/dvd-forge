use dvdforge_domain::{
    AssetId, AssetSource, Diagnostic, MediaAsset, MediaKind, MediaProbe, Project,
};
use dvdforge_media::Ffprobe;
use dvdforge_plan::BitratePlan;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub async fn collect_probes(
    project: &Project,
    project_dir: &Path,
    refresh: bool,
) -> (HashMap<AssetId, MediaProbe>, Vec<Diagnostic>) {
    let mut probes = HashMap::new();
    let mut warnings = Vec::new();
    // FFprobe is I/O-bound: probe up to four sources at once (as the renderer
    // does) and report results in project order.
    let ffprobe = Ffprobe::default();
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let mut pending = tokio::task::JoinSet::new();

    for (order, asset) in project.assets.iter().enumerate() {
        probes.insert(asset.id, asset.probe.clone());
        if asset.kind != MediaKind::Video {
            continue;
        }
        let should_probe = refresh || asset.probe.duration_ms.is_none();
        if !should_probe {
            continue;
        }
        let Some(path) = asset_path(project_dir, asset) else {
            warnings.push(Diagnostic::warning(
                "W_PROBE_PATH_MISSING",
                format!(
                    "No se pudo resolver la ruta del asset {}.",
                    asset.display_name
                ),
            ));
            continue;
        };
        if !path.is_file() {
            warnings.push(Diagnostic::warning(
                "W_PROBE_FILE_MISSING",
                format!(
                    "No existe el archivo {}; se usan los metadatos guardados.",
                    path.display()
                ),
            ));
            continue;
        }
        let id = asset.id;
        let ffprobe = ffprobe.clone();
        let permits = std::sync::Arc::clone(&permits);
        pending.spawn(async move {
            let _permit = permits.acquire_owned().await;
            let result = ffprobe.probe(&path).await;
            (order, id, path, result)
        });
    }

    let mut finished = Vec::new();
    while let Some(joined) = pending.join_next().await {
        if let Ok(result) = joined {
            finished.push(result);
        }
    }
    finished.sort_by_key(|(order, ..)| *order);
    for (_, id, path, result) in finished {
        match result {
            Ok(probe) => {
                probes.insert(id, probe);
            }
            Err(error) => {
                warnings.push(Diagnostic::warning(
                    "W_PROBE_FALLBACK",
                    format!(
                        "FFprobe falló para {}: {error}; se usan los metadatos guardados.",
                        path.display()
                    ),
                ));
            }
        }
    }

    (probes, warnings)
}

pub fn asset_path(project_dir: &Path, asset: &MediaAsset) -> Option<PathBuf> {
    match &asset.source {
        AssetSource::Project => asset
            .relative_path
            .as_ref()
            .map(|relative| project_dir.join(relative)),
        AssetSource::External { absolute_path } => Some(PathBuf::from(absolute_path)),
    }
}

pub fn format_duration(duration_ms: u64) -> String {
    let total_seconds = duration_ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub fn print_plan_summary(plan: &BitratePlan) {
    println!("Plan de bitrate");
    println!("  Modo: {:?}", plan.mode);
    println!("  Capacidad nominal: {} bytes", plan.capacity_bytes);
    println!(
        "  Duración de episodios: {}",
        format_duration(plan.total_duration_ms)
    );
    match &plan.avchd {
        Some(avchd) => println!(
            "  AVCHD: {} {}x{} @ {:.3} fps (entrelazado: {})",
            avchd.resolution.label(),
            avchd.width,
            avchd.height,
            avchd.frame_rate.as_f64(),
            avchd.interlaced
        ),
        None => println!("  Perfil: {:?}", plan.profile),
    }
    println!("  Video: {} kbps", plan.video_kbps);
    println!(
        "  Llenado: {:.1}% (objetivo 99.0%)",
        plan.fill_ppm as f64 / 10_000.0
    );
    println!("  Viable: {}", if plan.feasible { "sí" } else { "no" });
    for warning in &plan.warnings {
        println!("  Aviso {}: {}", warning.code, warning.message);
    }
}
