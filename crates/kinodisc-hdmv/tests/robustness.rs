//! Robustness tests: HDMV/BDMV parsers must never panic on arbitrary or
//! corrupted input (seeded PRNG, reproducible).

use kinodisc_hdmv::native::{build_movie_object, parse_pmt_from_m2ts_prefix};
use kinodisc_hdmv::{decode_command, movie_object_count, HdmvCommand};

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
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}

/// M2TS-shaped noise: 192-byte packets with a 0x47 sync at offset 4 and
/// PID 0 / PMT-like payloads, so the parser goes deep instead of bailing out.
fn m2ts_like(rng: &mut Rng, packets: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(packets * 192);
    for _ in 0..packets {
        let mut packet = rng.bytes(192);
        packet[4] = 0x47;
        if rng.below(3) == 0 {
            packet[5] = 0x40; // payload unit start, PID 0 (PAT)
            packet[6] = 0x00;
        } else if rng.below(3) == 0 {
            packet[5] = 0x41; // PMT on PID 0x100
            packet[6] = 0x00;
            packet[9] = 0x02; // table id 2
        }
        out.extend_from_slice(&packet);
    }
    out
}

#[test]
fn pmt_parser_never_panics() {
    for seed in 1..=2_000u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let input = if seed % 2 == 0 {
            let len = rng.below(4096);
            rng.bytes(len)
        } else {
            let packets = 1 + rng.below(40);
            let mut data = m2ts_like(&mut rng, packets);
            let cut = rng.below(data.len() + 1);
            data.truncate(cut);
            data
        };
        let outcome = std::panic::catch_unwind(|| parse_pmt_from_m2ts_prefix(&input));
        assert!(outcome.is_ok(), "parse_pmt panicked (seed {seed})");
    }
}

#[test]
fn movie_object_parser_never_panics() {
    let valid = build_movie_object(&[HdmvCommand::new(0x5000_0000, 0, 0)]).unwrap();
    for seed in 1..=2_000u64 {
        let mut rng = Rng(seed | 1);
        let input = if seed % 2 == 0 {
            let len = rng.below(256);
            rng.bytes(len)
        } else {
            let mut data = valid.clone();
            for _ in 0..1 + rng.below(8) {
                let at = rng.below(data.len());
                data[at] = rng.next() as u8;
            }
            data.truncate(rng.below(data.len() + 1));
            data
        };
        let outcome = std::panic::catch_unwind(|| movie_object_count(&input));
        assert!(outcome.is_ok(), "movie_object_count panicked (seed {seed})");
    }
}

#[test]
fn command_decoding_never_panics_and_round_trips() {
    let mut rng = Rng(7);
    for _ in 0..10_000 {
        let mut bytes = [0u8; 12];
        for byte in &mut bytes {
            *byte = rng.next() as u8;
        }
        let outcome = std::panic::catch_unwind(|| decode_command(&bytes));
        let command = outcome.expect("decode_command panicked");
        // Decoding is lossless for every 12-byte input.
        assert_eq!(command.to_bytes(), bytes);
    }
}
