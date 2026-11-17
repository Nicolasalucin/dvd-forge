//! `kinodisc-cli toolchain install <tool>` — one-click tool installation.

use kinodisc_toolchain::provision::{download_for, install_hint, provision};
use std::io::Write;
use std::process::ExitCode;

pub fn run(args: &[String]) -> ExitCode {
    let (Some(action), Some(tool)) = (args.first(), args.get(1)) else {
        eprintln!("Uso: kinodisc-cli toolchain install <ffmpeg|ffprobe>");
        return ExitCode::FAILURE;
    };
    if action != "install" {
        eprintln!("Uso: kinodisc-cli toolchain install <ffmpeg|ffprobe>");
        return ExitCode::FAILURE;
    }
    let Some(download) = download_for(tool) else {
        eprintln!("No hay descarga automática de {tool} para este sistema.");
        if let Some(hint) = install_hint(tool) {
            eprintln!("Instálalo con: {hint}");
        }
        return ExitCode::FAILURE;
    };
    println!(
        "Descargando {} {} ({:.0} MB) de {}",
        download.id,
        download.version,
        download.size as f64 / 1_000_000.0,
        download.url
    );
    let mut last = u64::MAX;
    let result = provision(download, |done, total| {
        let percent = done.saturating_mul(100) / total.max(1);
        if percent != last {
            last = percent;
            print!("\r  {percent:>3}%");
            let _ = std::io::stdout().flush();
        }
    });
    println!();
    match result {
        Ok(report) => {
            println!(
                "{} {} {} en {} (SHA-256 verificado). Licencia: {}",
                report.tool,
                report.version,
                if report.downloaded {
                    "instalado"
                } else {
                    "ya estaba instalado"
                },
                report.directory,
                report.license
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}
