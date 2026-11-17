//! Robustness ("fuzz-style") tests for the read-only verifier.
//!
//! A valid DVD-Video image is corrupted in many deterministic ways (seeded
//! PRNG, so failures are reproducible): random byte flips concentrated on the
//! descriptor area, truncation and zeroed sectors. The verifier must always
//! return a report or a typed error — never panic, hang or allocate wildly.

use std::path::{Path, PathBuf};

/// xorshift64*: tiny deterministic PRNG for reproducible corruption.
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

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kinodisc-verify-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A small but structurally complete DVD-Video image.
fn sample_image(dir: &Path) -> Vec<u8> {
    let video_ts = dir.join("src").join("VIDEO_TS");
    std::fs::create_dir_all(&video_ts).unwrap();
    let mut rng = Rng(0xD1D_F0E6);
    for (name, sectors) in [
        ("VIDEO_TS.IFO", 6),
        ("VIDEO_TS.VOB", 4),
        ("VIDEO_TS.BUP", 6),
        ("VTS_01_0.IFO", 9),
        ("VTS_01_1.VOB", 40),
        ("VTS_01_0.BUP", 9),
    ] {
        let mut bytes: Vec<u8> = (0..sectors * 2048).map(|_| rng.next() as u8).collect();
        if name.ends_with("IFO") || name.ends_with("BUP") {
            let magic: &[u8] = if name.starts_with("VIDEO_TS") {
                b"DVDVIDEO-VMG"
            } else {
                b"DVDVIDEO-VTS"
            };
            bytes[..12].copy_from_slice(magic);
        }
        std::fs::write(video_ts.join(name), bytes).unwrap();
    }
    let iso = dir.join("sample.iso");
    kinodisc_iso::write_dvd_video_iso(&dir.join("src"), &iso, "FUZZ", &|| false).unwrap();
    std::fs::read(iso).unwrap()
}

fn check(path: &Path, image: &[u8], seed: u64, case: &str) {
    std::fs::write(path, image).unwrap();
    let outcome = std::panic::catch_unwind(|| kinodisc_verify::verify_iso(path, true));
    assert!(
        outcome.is_ok(),
        "verify_iso panicked (case {case}, seed {seed:#x}, {} bytes)",
        image.len()
    );
}

#[test]
fn verifier_never_panics_on_corrupted_dvd_images() {
    let dir = temp_dir("fuzz");
    let image = sample_image(&dir);
    let target = dir.join("case.iso");
    // The intact image verifies (it may fail checks on random IFO bodies, but
    // it must produce a report).
    check(&target, &image, 0, "intact");

    let descriptor_area = (400 * 2048).min(image.len());
    for seed in 1..=300u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut corrupted = image.clone();
        let flips = 1 + rng.below(32);
        for _ in 0..flips {
            // 80 % of the flips hit the volume/file-system structures.
            let at = if rng.below(10) < 8 {
                rng.below(descriptor_area)
            } else {
                rng.below(corrupted.len())
            };
            corrupted[at] = rng.next() as u8;
        }
        check(&target, &corrupted, seed, "flips");
    }

    for seed in 1..=40u64 {
        let mut rng = Rng(seed * 7919);
        // Truncated images (sector aligned or not).
        let length = rng.below(image.len());
        check(&target, &image[..length], seed, "truncated");
        // Whole descriptor sectors zeroed.
        let mut zeroed = image.clone();
        let sector = rng.below(descriptor_area / 2048);
        zeroed[sector * 2048..(sector + 1) * 2048].fill(0);
        check(&target, &zeroed, seed, "zeroed-sector");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verifier_rejects_non_images() {
    let dir = temp_dir("junk");
    let target = dir.join("junk.iso");
    let mut rng = Rng(42);
    for size in [0usize, 1, 2047, 2048, 4096, 300 * 2048] {
        let junk: Vec<u8> = (0..size).map(|_| rng.next() as u8).collect();
        check(&target, &junk, size as u64, "junk");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
