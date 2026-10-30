use dvdforge_domain::{AspectRatio, DvdProfile, FrameRange, FrameRate, TvStandard};
use dvdforge_dvd::{AudioEncodeConfig, DvdEncoder, EncodeRequest, VideoEncodeConfig};
use std::path::PathBuf;

#[tokio::test]
#[ignore = "requiere ffmpeg y fixtures de medios generados"]
async fn encodes_a_short_episode_end_to_end() {
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/projects/assets/episode-01.mp4");
    if !input.is_file() {
        eprintln!("fixture no encontrado en {}; test omitido", input.display());
        return;
    }

    let output_dir =
        std::env::temp_dir().join(format!("dvdforge-encode-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&output_dir);
    std::fs::create_dir_all(&output_dir).expect("create temp dir");

    let request = EncodeRequest {
        input,
        output_dir: output_dir.clone(),
        stem: "episode-01".to_string(),
        video: VideoEncodeConfig {
            video_kbps: 3_000,
            maxrate_kbps: 8_000,
            profile: DvdProfile::D1,
            standard: TvStandard::Ntsc,
            aspect: AspectRatio::FourThree,
            gop_frames: 12,
            b_frames: 2,
            interlaced: false,
            top_field_first: true,
            scale_mode: dvdforge_domain::VideoScaleMode::Letterbox,
        },
        audio_tracks: vec![AudioEncodeConfig {
            source_stream: 1,
            bitrate_kbps: 192,
            channels: 2,
            sample_rate: 48_000,
            pre_filter: None,
            gain_db: 0.0,
            delay_ms: 0,
            duration_ms: 0,
        }],
        trim: Some(FrameRange { start: 0, end: 30 }),
        trim_start_ms: None,
        trim_end_ms: None,
        fps: FrameRate::NTSC,
        passlog: output_dir.join("episode-01.passlog"),
    };

    let outputs = DvdEncoder::default()
        .encode(&request)
        .await
        .expect("encode short clip");

    assert!(outputs.video_m2v.is_file());
    assert!(outputs.muxed_mpg.is_file());
    assert!(outputs.audio_ac3.iter().any(|path| path.is_file()));
    assert!(
        std::fs::metadata(&outputs.muxed_mpg)
            .expect("metadata")
            .len()
            > 0
    );

    let _ = std::fs::remove_dir_all(&output_dir);
}
