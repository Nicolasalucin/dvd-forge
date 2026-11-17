//! `kinodisc-cli batch queue.json [--json] [--resume] [--fail-fast]`
//!
//! Renders several projects one after another (optical authoring is memory
//! bound, so concurrency is always 1). The queue file:
//!
//! ```json
//! {
//!   "failFast": false,
//!   "items": [
//!     { "project": "serie/temporada-1.json", "output": "out/t1" },
//!     { "project": "serie/temporada-2.json", "output": "out/t2", "noProbe": true }
//!   ]
//! }
//! ```
//!
//! Relative paths are resolved against the queue file's folder. Progress is
//! saved to `<queue>.state.json` after every item, so `--resume` skips the
//! items that already produced their ISO.

use crate::render::{render, RenderArgs};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueFile {
    #[serde(default)]
    fail_fast: bool,
    items: Vec<QueueItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueItem {
    project: PathBuf,
    #[serde(default)]
    output: Option<PathBuf>,
    #[serde(default)]
    no_probe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemStatus {
    Pending,
    Done,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemReport {
    pub project: String,
    pub output: String,
    pub status: ItemStatus,
    #[serde(default)]
    pub iso: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchReport {
    pub items: Vec<ItemReport>,
    pub done: usize,
    pub failed: usize,
    pub skipped: usize,
    /// `W_BATCH_PARTIAL` when some items failed or were skipped.
    #[serde(default)]
    pub warning: Option<String>,
}

fn usage() -> ExitCode {
    eprintln!("Uso: kinodisc-cli batch <cola.json> [--json] [--resume] [--fail-fast]");
    ExitCode::FAILURE
}

fn state_path(queue: &Path) -> PathBuf {
    let mut name = queue.file_name().unwrap_or_default().to_os_string();
    name.push(".state.json");
    queue.with_file_name(name)
}

fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// Output folder when the item does not name one: `<queue dir>/out/<stem>`.
fn default_output(base: &Path, project: &Path) -> PathBuf {
    let stem = project
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "proyecto".to_string());
    base.join("out").join(stem)
}

fn load_state(path: &Path) -> Vec<ItemReport> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BatchReport>(&bytes).ok())
        .map(|report| report.items)
        .unwrap_or_default()
}

fn save_state(path: &Path, report: &BatchReport) {
    // Atomic replace: a crash never leaves a half-written state file.
    let temporary = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_vec_pretty(report) {
        if std::fs::write(&temporary, json).is_ok() {
            let _ = std::fs::rename(&temporary, path);
        }
    }
}

fn summarize(items: Vec<ItemReport>) -> BatchReport {
    let count = |status| items.iter().filter(|item| item.status == status).count();
    let (done, failed, skipped) = (
        count(ItemStatus::Done),
        count(ItemStatus::Failed),
        count(ItemStatus::Skipped),
    );
    let warning = (failed + skipped > 0).then(|| {
        format!("W_BATCH_PARTIAL: {done} completados, {failed} con error, {skipped} sin ejecutar.")
    });
    BatchReport {
        items,
        done,
        failed,
        skipped,
        warning,
    }
}

pub async fn run(args: &[String]) -> ExitCode {
    let Some(queue_arg) = args.iter().find(|arg| !arg.starts_with('-')) else {
        return usage();
    };
    let json = args.iter().any(|arg| arg == "--json");
    let resume = args.iter().any(|arg| arg == "--resume");
    let queue_path = PathBuf::from(queue_arg);
    let queue: QueueFile = match std::fs::read(&queue_path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
    {
        Ok(queue) => queue,
        Err(error) => {
            eprintln!(
                "E_BATCH_QUEUE: no se pudo leer la cola {}: {error}",
                queue_path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    if queue.items.is_empty() {
        eprintln!("E_BATCH_QUEUE: la cola no tiene elementos.");
        return ExitCode::FAILURE;
    }
    let fail_fast = queue.fail_fast || args.iter().any(|arg| arg == "--fail-fast");
    let base = queue_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let state_file = state_path(&queue_path);
    let previous = if resume {
        load_state(&state_file)
    } else {
        Vec::new()
    };

    let mut items: Vec<ItemReport> = queue
        .items
        .iter()
        .map(|item| {
            let project = resolve(&base, &item.project);
            let output = item
                .output
                .as_ref()
                .map(|output| resolve(&base, output))
                .unwrap_or_else(|| default_output(&base, &project));
            ItemReport {
                project: project.display().to_string(),
                output: output.display().to_string(),
                status: ItemStatus::Pending,
                iso: None,
                error: None,
                seconds: 0.0,
            }
        })
        .collect();

    let total = items.len();
    let mut stop = false;
    for (index, item) in queue.items.iter().enumerate() {
        let position = index + 1;
        if stop {
            items[index].status = ItemStatus::Skipped;
            continue;
        }
        let already_done = previous.iter().find(|done| {
            done.project == items[index].project
                && done.output == items[index].output
                && done.status == ItemStatus::Done
                && done
                    .iso
                    .as_deref()
                    .is_some_and(|iso| Path::new(iso).is_file())
        });
        if let Some(done) = already_done {
            items[index] = done.clone();
            if !json {
                println!(
                    "[{position}/{total}] ✓ ya completado: {}",
                    items[index].project
                );
            }
            continue;
        }
        if !json {
            println!();
            println!("[{position}/{total}] ▶ {}", items[index].project);
        }
        let started = Instant::now();
        let result = render(&RenderArgs {
            project: PathBuf::from(&items[index].project),
            output: PathBuf::from(&items[index].output),
            no_probe: item.no_probe,
            mode: None,
            avchd_folder: false,
        })
        .await;
        items[index].seconds = started.elapsed().as_secs_f64();
        match result {
            Ok(iso) => {
                items[index].status = ItemStatus::Done;
                items[index].iso = Some(iso.display().to_string());
                if !json {
                    println!("[{position}/{total}] ✓ {}", iso.display());
                }
            }
            Err(error) => {
                items[index].status = ItemStatus::Failed;
                items[index].error = Some(format!("E_BATCH_ITEM: {error}"));
                if !json {
                    eprintln!("[{position}/{total}] ✕ E_BATCH_ITEM: {error}");
                }
                stop = fail_fast;
            }
        }
        save_state(&state_file, &summarize(items.clone()));
    }

    let report = summarize(items);
    save_state(&state_file, &report);
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(error) => eprintln!("{error}"),
        }
    } else {
        println!();
        println!(
            "Cola terminada: {} completados, {} con error, {} sin ejecutar.",
            report.done, report.failed, report.skipped
        );
        if let Some(warning) = &report.warning {
            eprintln!("Aviso {warning}");
        }
    }
    if report.failed + report.skipped == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_against_the_queue_folder() {
        let base = Path::new("cola");
        assert_eq!(resolve(base, Path::new("a.json")), Path::new("cola/a.json"));
        assert_eq!(
            default_output(base, Path::new("cola/serie/t1.json")),
            Path::new("cola/out/t1")
        );
        assert_eq!(
            state_path(Path::new("cola/lote.json")),
            Path::new("cola/lote.json.state.json")
        );
    }

    #[test]
    fn summary_reports_partial_batches() {
        let item = |status| ItemReport {
            project: "p".into(),
            output: "o".into(),
            status,
            iso: None,
            error: None,
            seconds: 0.0,
        };
        let report = summarize(vec![
            item(ItemStatus::Done),
            item(ItemStatus::Failed),
            item(ItemStatus::Skipped),
        ]);
        assert_eq!((report.done, report.failed, report.skipped), (1, 1, 1));
        assert!(report.warning.unwrap().starts_with("W_BATCH_PARTIAL"));
        assert!(summarize(vec![item(ItemStatus::Done)]).warning.is_none());
    }

    #[test]
    fn queue_file_accepts_minimal_items() {
        let queue: QueueFile =
            serde_json::from_str(r#"{"items":[{"project":"a.json"}]}"#).expect("queue");
        assert!(!queue.fail_fast);
        assert!(queue.items[0].output.is_none());
    }
}
