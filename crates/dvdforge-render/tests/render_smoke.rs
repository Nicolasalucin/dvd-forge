use dvdforge_domain::{
    ButtonMotionConfig, ButtonShapeMask, DiscCapacity, FrameRate, JobId, MenuTemplateId,
    MenuTransition, MotionMenuConfig, SubtitleTrack, ThumbnailSpec, TransitionEasing,
};
use dvdforge_render::{render_project, render_project_with_layers};
use image::Rgba;
use std::path::PathBuf;
use tokio::sync::mpsc;

fn m2ts_has_pid(path: &std::path::Path, expected: u16) -> bool {
    let data = std::fs::read(path).expect("m2ts");
    data.as_chunks::<192>().0.iter().any(|packet| {
        // M2TS prefixes each 188-byte transport packet with a 4-byte ATS.
        packet.len() >= 192
            && packet[4] == 0x47
            && (u16::from(packet[5] & 0x1f) << 8 | u16::from(packet[6])) == expected
    })
}

#[tokio::test]
#[ignore = "render completo de ~1 minuto; requiere ffmpeg/dvdauthor/xorriso"]
async fn renders_fixture_to_iso() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = root.join("fixtures/projects/minimal.json");
    let project = dvdforge_project::load_project(project_path).expect("fixture");
    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!("dvdforge-render-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(128);
    let render_output = output.clone();
    let task =
        tokio::spawn(
            async move { render_project(project, project_dir, render_output, sender).await },
        );
    let mut stages = 0usize;
    while let Some(event) = receiver.recv().await {
        if matches!(event, dvdforge_jobs::JobEventKind::StageStarted { .. }) {
            stages += 1;
        }
    }
    let outcome = task.await.expect("task").expect("render");
    assert!(stages >= 5);
    assert!(PathBuf::from(&outcome.iso_path).is_file());
    assert!(outcome.iso_path.ends_with("disc.iso"));
    let _ = std::fs::remove_dir_all(output);
}

#[tokio::test]
#[ignore = "render completo de menú animado; requiere ffmpeg/dvdauthor/xorriso"]
async fn renders_motion_menu_with_video_button_to_iso() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = root.join("fixtures/projects/minimal.json");
    let mut project = dvdforge_project::load_project(project_path).expect("fixture");
    let menu = project
        .menus
        .iter_mut()
        .find(|menu| {
            menu.buttons.iter().any(|button| {
                matches!(
                    button.action,
                    dvdforge_domain::ButtonAction::PlayEpisode { .. }
                )
            })
        })
        .expect("menu with episode button");
    let menu_id = menu.id;
    let video_asset = menu
        .buttons
        .iter()
        .find_map(|button| match &button.action {
            dvdforge_domain::ButtonAction::PlayEpisode { episode } => project
                .episodes
                .iter()
                .find(|item| item.id == *episode)
                .map(|item| item.video),
            _ => None,
        })
        .expect("episode asset");
    menu.motion_config = Some(MotionMenuConfig::default());
    let button = menu
        .buttons
        .iter_mut()
        .find(|button| {
            matches!(
                button.action,
                dvdforge_domain::ButtonAction::PlayEpisode { .. }
            )
        })
        .expect("episode button");
    button.thumbnail = Some(ThumbnailSpec {
        asset: video_asset,
        in_frame: 0,
        duration_frames: 0,
        fps: FrameRate::NTSC,
    });
    button.motion_preview = Some(ButtonMotionConfig {
        source_start_ms: 0,
        loop_duration_ms: 5_000,
    });
    button.shape_mask = ButtonShapeMask::Circle;

    let layers =
        std::env::temp_dir().join(format!("dvdforge-motion-layers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&layers);
    std::fs::create_dir_all(&layers).expect("layers dir");
    image::RgbaImage::from_pixel(720, 480, Rgba([24, 48, 72, 255]))
        .save(layers.join(format!("{menu_id}.png")))
        .expect("background layer");
    image::RgbaImage::from_pixel(720, 480, Rgba([0, 0, 0, 0]))
        .save(layers.join(format!("labels-{menu_id}.png")))
        .expect("labels layer");

    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!("dvdforge-motion-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(128);
    let render_output = output.clone();
    let task = tokio::spawn(async move {
        render_project_with_layers(
            project,
            project_dir,
            render_output,
            JobId::new(),
            Some(layers),
            sender,
        )
        .await
    });
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("motion render");
    assert!(PathBuf::from(&outcome.iso_path).is_file());
    let menu_dir = output.join("menus").join(menu_id.to_string());
    assert!(menu_dir.join("menu_base.mpg").is_file());
    assert!(menu_dir.join("menu_sub.mpg").is_file());
    let base_size = std::fs::metadata(menu_dir.join("menu_base.mpg"))
        .expect("motion menu metadata")
        .len();
    assert!(base_size > 0);
    let _ = std::fs::remove_dir_all(output);
}

#[tokio::test]
#[ignore = "render completo con spumux de subtítulos; requiere ffmpeg/dvdauthor/xorriso"]
async fn renders_fixture_with_external_subtitle_to_iso() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = root.join("fixtures/projects/minimal.json");
    let mut project = dvdforge_project::load_project(project_path).expect("fixture");
    let srt = std::env::temp_dir().join(format!(
        "dvdforge-render-subtitle-{}.srt",
        std::process::id()
    ));
    std::fs::write(
        &srt,
        "1\n00:00:00,500 --> 00:00:02,000\nDVD Forge subtítulo de prueba\n\n",
    )
    .expect("srt");
    project.episodes[0].subtitles = vec![SubtitleTrack {
        source_stream: 0,
        source_path: Some(srt.display().to_string()),
        language: "spa".to_string(),
        label: "Español".to_string(),
        delay_ms: 0,
        is_forced: false,
        is_default: true,
        bitmap: false,
        style: Default::default(),
    }];
    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!(
        "dvdforge-render-subtitle-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(128);
    let render_output = output.clone();
    let task =
        tokio::spawn(
            async move { render_project(project, project_dir, render_output, sender).await },
        );
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("render with subtitles");
    assert!(PathBuf::from(&outcome.iso_path).is_file());
    let script =
        std::fs::read_to_string(output.join("xml").join("dvdauthor.xml")).expect("dvdauthor xml");
    assert!(script.contains("<subpicture"));
    let _ = std::fs::remove_file(srt);
    let _ = std::fs::remove_dir_all(output);
}

#[tokio::test]
#[ignore = "render BD-25 con menú HDMV IG; requiere ffmpeg/tsMuxeR y fixtures"]
async fn renders_bd25_with_native_hdmv_menu() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = root.join("fixtures/projects/bd25-menu.json");
    let project = dvdforge_project::load_project(project_path).expect("fixture");
    let project_dir = root.join("fixtures/projects");
    let output =
        std::env::temp_dir().join(format!("dvdforge-render-hdmv-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(256);
    let render_output = output.clone();
    let task =
        tokio::spawn(
            async move { render_project(project, project_dir, render_output, sender).await },
        );
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("render");
    assert!(PathBuf::from(&outcome.iso_path).is_file());
    assert!(output.join("avchd/BDMV/PLAYLIST/00001.mpls").is_file());
    assert!(output.join("avchd/BDMV/CLIPINF/00001.clpi").is_file());
    assert!(output.join("avchd/BDMV/STREAM/00001.m2ts").is_file());
    assert!(output.join("menus/clip-00/menu.meta").is_file());
    let _ = std::fs::remove_dir_all(output);
}

#[tokio::test]
#[ignore = "render AVCHD DVD5 con menú HDMV IG; requiere ffmpeg/tsMuxeR y fixtures"]
async fn renders_avchd_dvd5_with_native_hdmv_menu() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = root.join("fixtures/projects/bd25-menu.json");
    let mut project = dvdforge_project::load_project(project_path).expect("fixture");
    project.disc.capacity = DiscCapacity::Dvd5;
    project.disc.volume_label = "AVCHD_MENU_DEMO".to_string();
    project.disc.video = dvdforge_domain::VideoConfig::Avchd(dvdforge_domain::AvchdVideoConfig {
        resolution: dvdforge_domain::AvchdResolution::R720p,
        frame_rate: FrameRate::FPS_23_976,
        level: dvdforge_domain::H264Level::Level41,
        vbv_maxrate_kbps: 24_000,
        vbv_bufsize_kbps: 24_000,
        ..dvdforge_domain::AvchdVideoConfig::default()
    });
    project.budget.max_video_kbps = 24_000;
    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!(
        "dvdforge-render-avchd-menu-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(256);
    let render_output = output.clone();
    let task =
        tokio::spawn(
            async move { render_project(project, project_dir, render_output, sender).await },
        );
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("render");
    assert!(PathBuf::from(&outcome.iso_path).is_file());
    assert!(output.join("avchd/BDMV/PLAYLIST/00001.mpls").is_file());
    assert!(output.join("avchd/BDMV/STREAM/00001.m2ts").is_file());
    let _ = std::fs::remove_dir_all(output);
}

#[tokio::test]
#[ignore = "render BD-25 con menús HDMV anidados y audio; requiere ffmpeg/tsMuxeR/libbluray"]
async fn renders_bd25_with_nested_hdmv_menus() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = root.join("fixtures/projects/bd25-nested-menu.json");
    let project = dvdforge_project::load_project(project_path).expect("fixture");
    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!(
        "dvdforge-render-hdmv-nested-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(256);
    let render_output = output.clone();
    let task = tokio::spawn(async move {
        dvdforge_render::render_project(project, project_dir, render_output, sender).await
    });
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("nested render");
    assert!(PathBuf::from(&outcome.iso_path).is_file());

    let bdmv = output.join("avchd/BDMV");
    for playlist in ["00001.mpls", "00002.mpls"] {
        assert!(bdmv.join("PLAYLIST").join(playlist).is_file(), "{playlist}");
    }
    for stream in ["00001.m2ts", "00002.m2ts"] {
        let path = bdmv.join("STREAM").join(stream);
        assert!(path.is_file(), "{stream}");
        assert!(m2ts_has_pid(&path, 0x1011), "{stream} sin vídeo H.264");
        assert!(m2ts_has_pid(&path, 0x1100), "{stream} sin audio AC-3");
    }
    // Both menu clips are encoded with AC-3, including the silent fallback for
    // the menu that has no configured music.
    for clip in ["clip-00/menu.ac3", "clip-01/menu.ac3"] {
        let path = output.join("menus").join(clip);
        assert!(path.is_file(), "{}", path.display());
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
    }
    // The root menu uses a video background, so its H.264 clip must be
    // non-trivial and High profile.
    let video = output.join("menus/clip-00/menu.h264");
    assert!(std::fs::metadata(&video).unwrap().len() > 10_000);

    let movie_object = std::fs::read(bdmv.join("MovieObject.bdmv")).expect("movie object");
    let objects = dvdforge_hdmv::movie_object_count(&movie_object).expect("object count");
    assert!(
        objects >= 3,
        "se esperaban los objetos base + 2 menús, hay {objects}"
    );

    let _ = std::fs::remove_dir_all(output);
}

#[tokio::test]
#[ignore = "render DVD con región; requiere ffmpeg/dvdauthor/xorriso"]
async fn renders_dvd_with_region_mask() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = root.join("fixtures/projects/minimal.json");
    let mut project = dvdforge_project::load_project(project_path).expect("fixture");
    project.disc.region_mask = 0b0000_0101;
    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!(
        "dvdforge-render-region-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(128);
    let render_output = output.clone();
    let task =
        tokio::spawn(
            async move { render_project(project, project_dir, render_output, sender).await },
        );
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("region render");
    assert!(PathBuf::from(&outcome.iso_path).is_file());
    for name in ["VIDEO_TS.IFO", "VIDEO_TS.BUP"] {
        let bytes = std::fs::read(output.join("dvd/VIDEO_TS").join(name)).expect("ifo");
        assert_eq!(bytes[0x23], 0xFA, "{name} región prohibida");
    }
    let _ = std::fs::remove_dir_all(output);
}

#[tokio::test]
#[ignore = "render AVCHD sin menús con UDF metadata; requiere ffmpeg/tsMuxeR"]
async fn renders_menu_less_avchd_with_metadata_udf() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = root.join("fixtures/projects/avchd-dvd5.json");
    let project = dvdforge_project::load_project(project_path).expect("fixture");
    assert!(project.menus.is_empty());
    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!(
        "dvdforge-render-avchd-nomenu-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(128);
    let render_output = output.clone();
    let task =
        tokio::spawn(
            async move { render_project(project, project_dir, render_output, sender).await },
        );
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("render");
    let iso = std::fs::read(&outcome.iso_path).expect("iso");
    let mut metadata = false;
    let mut mirror = false;
    for block in iso.as_chunks::<2_048>().0.iter().take(800) {
        if u16::from_le_bytes([block[0], block[1]]) == 266 {
            match block[27] {
                250 => metadata = true,
                251 => mirror = true,
                _ => {}
            }
        }
    }
    assert!(metadata && mirror, "UDF sin Metadata File + Mirror");
    assert!(output.join("avchd/BDMV/index.bdmv").is_file());
    let _ = std::fs::remove_dir_all(output);
}

#[tokio::test]
#[ignore = "render de menú DVD con transición; requiere ffmpeg/dvdauthor/xorriso"]
async fn renders_dvd_motion_transition() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = root.join("fixtures/projects/minimal.json");
    let mut project = dvdforge_project::load_project(project_path).expect("fixture");
    let menu = project.menus.first_mut().expect("menu");
    let asset = project
        .episodes
        .first()
        .map(|episode| episode.video)
        .expect("episode");
    menu.transition = Some(MenuTransition {
        asset,
        duration_ms: 1_500,
        easing: TransitionEasing::EaseInOut,
    });
    menu.motion_config = Some(MotionMenuConfig::default());
    if let Some(button) = menu.buttons.first_mut() {
        button.motion_preview = Some(ButtonMotionConfig {
            source_start_ms: 0,
            loop_duration_ms: 2_000,
        });
    }
    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!(
        "dvdforge-render-transition-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(128);
    let render_output = output.clone();
    let task =
        tokio::spawn(
            async move { render_project(project, project_dir, render_output, sender).await },
        );
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("transition render");
    assert!(PathBuf::from(&outcome.iso_path).is_file());
    let report = dvdforge_verify::verify_iso(std::path::Path::new(&outcome.iso_path), false)
        .expect("verify");
    assert_ne!(
        report.verdict,
        dvdforge_verify::report::VerifyVerdict::Fail,
        "{:?}",
        report.checks
    );
    assert!(report
        .checks
        .iter()
        .any(|check| check.id == "DVD-SPU-BUTTONS"
            && check.status == dvdforge_verify::report::CheckStatus::Pass));
    let _ = std::fs::remove_dir_all(output);
}

#[tokio::test]
#[ignore = "render de menú BD-25 con transición; requiere ffmpeg/tsMuxeR/libbluray"]
async fn renders_bd25_motion_transition() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = root.join("fixtures/projects/bd25-menu.json");
    let mut project = dvdforge_project::load_project(project_path).expect("fixture");
    let asset = project
        .episodes
        .first()
        .map(|episode| episode.video)
        .expect("episode");
    let menu = project.menus.first_mut().expect("menu");
    menu.transition = Some(MenuTransition {
        asset,
        duration_ms: 1_500,
        easing: TransitionEasing::Linear,
    });
    menu.motion_config = Some(MotionMenuConfig::default());
    if let Some(button) = menu.buttons.first_mut() {
        button.motion_preview = Some(ButtonMotionConfig {
            source_start_ms: 0,
            loop_duration_ms: 2_000,
        });
    }
    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!(
        "dvdforge-render-bd-transition-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(128);
    let render_output = output.clone();
    let task = tokio::spawn(async move {
        dvdforge_render::render_project(project, project_dir, render_output, sender).await
    });
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("bd transition render");
    assert!(PathBuf::from(&outcome.iso_path).is_file());
    let report = dvdforge_verify::verify_iso(std::path::Path::new(&outcome.iso_path), false)
        .expect("verify");
    assert_ne!(
        report.verdict,
        dvdforge_verify::report::VerifyVerdict::Fail,
        "{:?}",
        report.checks
    );
    for id in ["BDMV-IG-BUTTONS", "BDMV-IG-NAV", "BDMV-MOTION-THUMB"] {
        assert!(
            report.checks.iter().any(|check| check.id == id
                && check.status == dvdforge_verify::report::CheckStatus::Pass),
            "missing {id}: {:?}",
            report.checks
        );
    }
    let bdmv = output.join("avchd");
    if let Ok(output) = std::process::Command::new("bd_info").arg(&bdmv).output() {
        assert!(output.status.success(), "bd_info rejected {:?}", bdmv);
    }
    let _ = std::fs::remove_dir_all(output);
}
#[tokio::test]
#[ignore = "render DVD multipágina con plantilla; requiere ffmpeg/dvdauthor/xorriso"]
async fn renders_dvd_multipage_template() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut project = dvdforge_project::load_project(root.join("fixtures/projects/minimal.json"))
        .expect("fixture");
    let source = project.episodes[0].clone();
    project.episodes = (0..12)
        .map(|index| {
            let mut episode = source.clone();
            episode.id = dvdforge_domain::EpisodeId::new();
            episode.number = (index + 1) as u16;
            episode.title = format!("Episode {:02}", index + 1);
            episode.trim_start_ms = Some(0);
            episode.trim_end_ms = Some(3_000 + index as u64 * 100);
            episode
        })
        .collect();
    let applied =
        dvdforge_menu::templates::apply_template(&project, MenuTemplateId::MultiPageTvGrid)
            .expect("template");
    project.menus = applied
        .menus
        .into_iter()
        .filter(|menu| menu.template.is_some())
        .collect();
    project.root_menu = applied.root_menu;
    project.play_all.order = project.episodes.iter().map(|episode| episode.id).collect();
    let page_ids: Vec<_> = project.menus.iter().map(|menu| menu.id).collect();
    for menu in &mut project.menus {
        for button in &mut menu.buttons {
            if let dvdforge_domain::ButtonAction::GotoMenu { menu: target } = button.action {
                if !page_ids.contains(&target) {
                    button.action = dvdforge_domain::ButtonAction::Noop;
                }
            }
        }
        menu.timing.duration_ms = 5_000;
    }
    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!(
        "dvdforge-render-template-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(256);
    let render_output = output.clone();
    let task = tokio::spawn(async move {
        dvdforge_render::render_project(project, project_dir, render_output, sender).await
    });
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("template render");
    assert!(PathBuf::from(&outcome.iso_path).is_file());
    let report = dvdforge_verify::verify_iso(std::path::Path::new(&outcome.iso_path), false)
        .expect("verify");
    assert_ne!(
        report.verdict,
        dvdforge_verify::report::VerifyVerdict::Fail,
        "{:?}",
        report.checks
    );
    assert!(report
        .checks
        .iter()
        .any(|check| check.id == "DVD-SPU-BUTTONS"
            && check.status == dvdforge_verify::report::CheckStatus::Pass));
    let _ = std::fs::remove_dir_all(output);
}

#[tokio::test]
#[ignore = "render BD-25 multipágina con plantilla; requiere ffmpeg/tsMuxeR/libbluray"]
async fn renders_bd25_multipage_template() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut project = dvdforge_project::load_project(root.join("fixtures/projects/bd25-menu.json"))
        .expect("fixture");
    let source = project.episodes[0].clone();
    project.episodes = (0..12)
        .map(|index| {
            let mut episode = source.clone();
            episode.id = dvdforge_domain::EpisodeId::new();
            episode.number = (index + 1) as u16;
            episode.title = format!("Episode {:02}", index + 1);
            episode.trim_start_ms = Some(0);
            episode.trim_end_ms = Some(3_000 + index as u64 * 100);
            episode
        })
        .collect();
    let applied = dvdforge_menu::templates::apply_template(&project, MenuTemplateId::CinemaDark)
        .expect("template");
    project.menus = applied
        .menus
        .into_iter()
        .filter(|menu| menu.template.is_some())
        .collect();
    project.root_menu = applied.root_menu;
    project.play_all.order = project.episodes.iter().map(|episode| episode.id).collect();
    let page_ids: Vec<_> = project.menus.iter().map(|menu| menu.id).collect();
    for menu in &mut project.menus {
        for button in &mut menu.buttons {
            if let dvdforge_domain::ButtonAction::GotoMenu { menu: target } = button.action {
                if !page_ids.contains(&target) {
                    button.action = dvdforge_domain::ButtonAction::Noop;
                }
            }
        }
        menu.timing.duration_ms = 5_000;
    }
    let project_dir = root.join("fixtures/projects");
    let output = std::env::temp_dir().join(format!(
        "dvdforge-render-bd-template-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&output);
    let (sender, mut receiver) = mpsc::channel(256);
    let render_output = output.clone();
    let task = tokio::spawn(async move {
        dvdforge_render::render_project(project, project_dir, render_output, sender).await
    });
    while receiver.recv().await.is_some() {}
    let outcome = task.await.expect("task").expect("bd template render");
    assert!(PathBuf::from(&outcome.iso_path).is_file());
    let report = dvdforge_verify::verify_iso(std::path::Path::new(&outcome.iso_path), false)
        .expect("verify");
    assert_ne!(
        report.verdict,
        dvdforge_verify::report::VerifyVerdict::Fail,
        "{:?}",
        report.checks
    );
    for id in ["BDMV-IG-BUTTONS", "BDMV-IG-NAV"] {
        assert!(
            report.checks.iter().any(|check| check.id == id
                && check.status == dvdforge_verify::report::CheckStatus::Pass),
            "missing {id}: {:?}",
            report.checks
        );
    }
    let bdmv = output.join("avchd");
    if let Ok(result) = std::process::Command::new("bd_info").arg(&bdmv).output() {
        assert!(result.status.success(), "bd_info rejected {:?}", bdmv);
    }
    let _ = std::fs::remove_dir_all(output);
}
