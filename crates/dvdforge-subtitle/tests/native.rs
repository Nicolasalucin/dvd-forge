use dvdforge_subtitle::{
    cues::{parse_text, SubtitleFormat},
    pgs::render_pgs,
    raster::RasterOptions,
    spumux::render_spumux_assets,
    SubtitleStyle,
};

#[test]
fn parses_srt_ass_and_vtt() {
    let srt = "1\n00:00:01,000 --> 00:00:03,000\nHello <i>world</i>\n";
    let parsed = parse_text(SubtitleFormat::Srt, srt).expect("srt");
    assert_eq!(parsed.cues.len(), 1);
    assert_eq!(parsed.cues[0].text, "Hello world");

    let ass = "[Script Info]\nScriptType: v4.00+\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, OutlineColour, BackColour, Bold, Italic, Alignment\nStyle: Default,Arial,48,&H00FFFFFF,&H00000000,&H00000000,0,0,2\n[Events]\nFormat: Layer, Start, End, Style, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Default,Hello\n";
    let parsed = parse_text(SubtitleFormat::Ass, ass).expect("ass");
    assert_eq!(parsed.cues.len(), 1);
    assert_eq!(parsed.cues[0].style.font_size_px, Some(48.0));

    let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nHello\n";
    let parsed = parse_text(SubtitleFormat::Vtt, vtt).expect("vtt");
    assert_eq!(parsed.cues.len(), 1);
}

#[test]
fn renders_pgs_and_spumux_assets() {
    let parsed = parse_text(
        SubtitleFormat::Srt,
        "1\n00:00:01,000 --> 00:00:03,000\nDVD Forge\n",
    )
    .expect("srt");
    let style = SubtitleStyle::default();
    let options = RasterOptions {
        width: 1280,
        height: 720,
        style: &style,
    };
    let pgs = render_pgs(&parsed.cues, &options).expect("pgs");
    assert!(pgs.starts_with(b"PG"));
    assert!(pgs.windows(2).any(|window| window == b"PG"));

    let dir = std::env::temp_dir().join(format!("dvdforge-subtitle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let assets = render_spumux_assets(&parsed.cues, &options, &dir, "track").expect("spumux");
    assert!(assets.xml.contains("<spu "));
    assert_eq!(assets.png_paths.len(), 1);
    let image = image::open(&assets.png_paths[0]).expect("png").to_rgba8();
    let mut colors = std::collections::HashSet::new();
    for pixel in image.pixels() {
        colors.insert(pixel.0);
    }
    assert!(colors.len() <= 4, "{} colors", colors.len());
    let _ = std::fs::remove_dir_all(dir);
}
