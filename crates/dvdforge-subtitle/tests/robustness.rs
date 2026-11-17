//! Robustness tests: subtitle parsers must never panic on malformed files
//! (seeded PRNG, reproducible). Inputs mix real SRT/ASS/VTT syntax with noise
//! so the parsers reach their deeper branches.

use dvdforge_subtitle::{parse_path, SubtitleFormat};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit.max(1) as u64) as usize
    }
}

const FRAGMENTS: &[&str] = &[
    "1\n",
    "2\n",
    "00:00:01,000 --> 00:00:02,500\n",
    "00:00:01.000 --> 00:00:02.500\n",
    "99:99:99,999 --> 00:00:00,000\n",
    "-->",
    " --> ",
    "\n\n",
    "Hola\n",
    "<i>x</i>",
    "{\\an8}",
    "{\\pos(10,20)}",
    "{\\b1}",
    "{\\c&H00FF00&}",
    "\\N",
    "WEBVTT\n\n",
    "NOTE comment\n\n",
    "STYLE\n::cue { color: red }\n\n",
    "[Script Info]\n",
    "[V4+ Styles]\n",
    "Format: Name, Fontname, Fontsize, PrimaryColour, Alignment\n",
    "Style: Default,Arial,48,&H00FFFFFF,2\n",
    "[Events]\n",
    "Format: Layer, Start, End, Style, Text\n",
    "Dialogue: 0,0:00:01.00,0:00:02.00,Default,Hi\n",
    "Dialogue: 0,9:99:99.99,,,\n",
    ",,,,,,,,,",
    "ñ",
    "日本語",
    "\u{feff}",
    "\r\n",
    "\0",
    "align:start position:10%",
    "line:-1",
    "&H",
    "{",
    "}",
    ":",
    ".",
    ",",
];

fn sample(rng: &mut Rng) -> String {
    let mut text = String::new();
    for _ in 0..rng.below(80) {
        if rng.below(6) == 0 {
            // Random printable/control character.
            text.push(char::from_u32(rng.below(0x3000) as u32).unwrap_or('?'));
        } else {
            text.push_str(FRAGMENTS[rng.below(FRAGMENTS.len())]);
        }
    }
    text
}

#[test]
fn subtitle_parsers_never_panic() {
    let dir = std::env::temp_dir().join(format!("dvdforge-sub-fuzz-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for seed in 1..=1_500u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let text = sample(&mut rng);
        for (format, extension) in [
            (SubtitleFormat::Srt, "srt"),
            (SubtitleFormat::Ass, "ass"),
            (SubtitleFormat::Vtt, "vtt"),
        ] {
            let path = dir.join(format!("case.{extension}"));
            std::fs::write(&path, &text).unwrap();
            let outcome = std::panic::catch_unwind(|| parse_path(&path));
            assert!(
                outcome.is_ok(),
                "{format:?} parser panicked (seed {seed}): {:?}",
                text.chars().take(120).collect::<String>()
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
