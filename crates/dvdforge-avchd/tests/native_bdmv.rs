//! Integration proof that the native FFmpeg BDMV fallback produces a
//! structure accepted by the independent verifier, including PGS pass-through.

use dvdforge_avchd::native::{NativeBdmvMuxer, NativeMovieRequest, NativeMovieSubtitle};
use dvdforge_hdmv::dummy_pgs;
use dvdforge_iso::BdmvIsoBuilder;
use dvdforge_toolchain::resolve_binary;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "dvdforge-native-{label}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn check_bd_info(root: &Path) {
    match std::process::Command::new("bd_info").arg(root).output() {
        Ok(output) if output.status.success() => {}
        Ok(output) => panic!(
            "bd_info rejected {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("bd_info failed: {error}"),
    }
}

fn run(ffmpeg: &Path, args: &[&str]) {
    let output = std::process::Command::new(ffmpeg)
        .args(args)
        .output()
        .expect("ffmpeg");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn native_movie_with_pgs_is_verifiable() {
    let ffmpeg = resolve_binary("ffmpeg");
    if ffmpeg.components().count() == 1 || !ffmpeg.is_file() {
        eprintln!("FFmpeg is unavailable; skipping native BDMV integration test");
        return;
    }
    let root = temp_root("pgs");
    let sup = root.join("dummy.sup");
    std::fs::write(&sup, dummy_pgs(320, 180, 2).expect("dummy pgs")).unwrap();
    let combined = root.join("combined.mkv");
    run(
        &ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24000/1001:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-f",
            "sup",
            "-i",
            sup.to_str().unwrap(),
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-map",
            "2:s:0",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "ac3",
            "-c:s",
            "copy",
            "-metadata:s:s:0",
            "language=spa",
            combined.to_str().unwrap(),
        ],
    );
    let bdmv_root = root.join("avchd");
    let report = NativeBdmvMuxer::new(&ffmpeg)
        .mux_movie(&NativeMovieRequest {
            video: &combined,
            output_dir: &bdmv_root,
            duration_ms: 2_000,
            chapters_ms: &[0, 1_000],
            audio_language: "eng",
            subtitles: &[NativeMovieSubtitle {
                path: None,
                language: "spa",
            }],
        })
        .await
        .expect("native movie");
    assert_eq!(report.movie_objects, 1);
    assert!(report
        .streams
        .iter()
        .any(|stream| stream.stream_type == 0x90 && stream.pid == 0x1200));
    assert!(bdmv_root.join("BDMV/PLAYLIST/00000.mpls").is_file());
    check_bd_info(&bdmv_root);

    let iso = root.join("disc.iso");
    BdmvIsoBuilder::default()
        .build(&bdmv_root, &iso, "DVD_FORGE_TEST")
        .await
        .expect("bdmv iso");
    let verified = dvdforge_verify::verify_iso(&iso, false).expect("verify");
    assert_ne!(
        verified.verdict,
        dvdforge_verify::report::VerifyVerdict::Fail,
        "{:?}",
        verified.checks
    );
    for id in ["BDMV-M2TS", "BDMV-PGS", "BDMV-MPLS", "BDMV-CLPI"] {
        assert!(
            verified.checks.iter().any(|check| check.id == id
                && check.status == dvdforge_verify::report::CheckStatus::Pass),
            "missing {id}: {:?}",
            verified.checks
        );
    }
    let _ = std::fs::remove_dir_all(root);
}
#[tokio::test]
async fn native_menu_ig_is_verifiable() {
    use dvdforge_hdmv::{
        compile_ig_pes_packets, frame_rate_code, HdmvButton, HdmvButtonState, HdmvMenu, RgbaBitmap,
    };
    let ffmpeg = resolve_binary("ffmpeg");
    if ffmpeg.components().count() == 1 || !ffmpeg.is_file() {
        eprintln!("FFmpeg is unavailable; skipping native menu integration test");
        return;
    }
    let root = temp_root("menu");
    let combined = root.join("combined.mkv");
    run(
        &ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24000/1001:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "ac3",
            combined.to_str().unwrap(),
        ],
    );
    let bdmv_root = root.join("avchd");
    NativeBdmvMuxer::new(&ffmpeg)
        .mux_movie(&NativeMovieRequest {
            video: &combined,
            output_dir: &bdmv_root,
            duration_ms: 2_000,
            chapters_ms: &[0],
            audio_language: "eng",
            subtitles: &[],
        })
        .await
        .expect("native movie");

    let normal = RgbaBitmap::solid(120, 40, [20, 40, 70, 255]).unwrap();
    let selected = RgbaBitmap::solid(120, 40, [255, 255, 255, 255]).unwrap();
    let activated = RgbaBitmap::solid(120, 40, [34, 211, 238, 255]).unwrap();
    let menu = HdmvMenu {
        width: 320,
        height: 180,
        frame_rate_code: frame_rate_code(23.976),
        background: Some(RgbaBitmap::solid(320, 180, [8, 12, 24, 255]).unwrap()),
        buttons: vec![HdmvButton {
            id: 1,
            x: 20,
            y: 20,
            up: None,
            down: None,
            left: None,
            right: None,
            commands: vec![dvdforge_hdmv::jump_title(1)],
            state: HdmvButtonState {
                normal,
                selected,
                activated,
            },
        }],
        default_selected: Some(1),
    };
    let ig = compile_ig_pes_packets(&menu, 54_000_000).expect("ig");

    let menu_video = root.join("menu.h264");
    run(
        &ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x180:r=24000/1001:d=2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-f",
            "h264",
            menu_video.to_str().unwrap(),
        ],
    );
    let menu_audio = root.join("menu.ac3");
    run(
        &ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-t",
            "2",
            "-c:a",
            "ac3",
            menu_audio.to_str().unwrap(),
        ],
    );
    let muxer = NativeBdmvMuxer::new(&ffmpeg);
    let report = muxer
        .attach_menu(&dvdforge_avchd::native::NativeMenuRequest {
            bdmv_dir: &bdmv_root,
            work_dir: &root.join("menu-work"),
            video_h264: &menu_video,
            audio_ac3: &menu_audio,
            ig_pes_packets: &ig,
            width: 320,
            height: 180,
            duration_ms: 2_000,
            clip_id: 1,
            playlist_id: 1,
            expected_object: 1,
        })
        .await
        .expect("native menu");
    assert_eq!(report.object, 1);
    muxer.set_root_menu(&bdmv_root, 1).expect("root menu");
    check_bd_info(&bdmv_root);

    let iso = root.join("disc-menu.iso");
    BdmvIsoBuilder::default()
        .build(&bdmv_root, &iso, "DVD_FORGE_MENU_TEST")
        .await
        .expect("bdmv iso");
    let verified = dvdforge_verify::verify_iso(&iso, false).expect("verify");
    assert_ne!(
        verified.verdict,
        dvdforge_verify::report::VerifyVerdict::Fail,
        "{:?}",
        verified.checks
    );
    for id in ["BDMV-M2TS", "BDMV-IG", "BDMV-MPLS", "BDMV-CLPI"] {
        assert!(
            verified.checks.iter().any(|check| check.id == id
                && check.status == dvdforge_verify::report::CheckStatus::Pass),
            "missing {id}: {:?}",
            verified.checks
        );
    }
    let _ = std::fs::remove_dir_all(root);
}
