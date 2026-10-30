//! Bounded, allocation-stable I/O helpers shared by the verifier and render
//! pipelines. Every large source is processed through one reusable buffer; no
//! function in this module allocates in proportion to the input length.

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

/// Reusable payload buffer for streaming copies and hashes. Kept well below the
/// 1 MiB upper bound so multiple concurrent jobs remain inside the 32-bit RSS
/// ceiling.
pub const STREAM_BUFFER_BYTES: usize = 256 * 1024;

/// Copies `reader` to `writer` with a single fixed-size buffer and returns the
/// number of bytes copied. The `u64` return is deliberately independent of the
/// host `usize` so >4 GiB virtual/sparse streams work on i686.
pub fn copy_bounded<R: Read, W: Write>(reader: &mut R, writer: &mut W) -> io::Result<u64> {
    let mut buffer = vec![0u8; STREAM_BUFFER_BYTES];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("stream length overflow"))?;
    }
    Ok(total)
}

/// Streaming SHA-256 over any reader. No whole-file allocation is performed.
pub fn hash_reader<R: Read>(mut reader: R) -> io::Result<String> {
    let mut buffer = vec![0u8; STREAM_BUFFER_BYTES];
    let mut hasher = Sha256::new();
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Streaming SHA-256 over a file, used by cache fingerprints and subtitle
/// metadata hashing.
pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    hash_reader(&mut file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct VirtualBytes {
        remaining: u64,
        max_request: usize,
        produced: u64,
    }

    impl VirtualBytes {
        fn new(total: u64) -> Self {
            Self {
                remaining: total,
                max_request: 0,
                produced: 0,
            }
        }
    }

    impl Read for VirtualBytes {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.max_request = self.max_request.max(buffer.len());
            if self.remaining == 0 {
                return Ok(0);
            }
            let count = usize::try_from(self.remaining.min(buffer.len() as u64))
                .map_err(|_| io::Error::other("read size overflow"))?;
            buffer[..count].fill(0);
            self.remaining -= count as u64;
            self.produced += count as u64;
            Ok(count)
        }
    }

    #[test]
    fn copy_bounded_streams_more_than_four_gib_with_one_buffer() {
        let total = u64::from(u32::MAX) + 4096;
        let mut reader = VirtualBytes::new(total);
        let copied = copy_bounded(&mut reader, &mut io::sink()).expect("copy");
        assert_eq!(copied, total);
        assert!(copied > u64::from(u32::MAX));
        assert!(reader.max_request <= STREAM_BUFFER_BYTES);
    }

    #[test]
    fn hash_reader_matches_small_reference() {
        let digest = hash_reader(Cursor::new(b"dvdforge".to_vec())).expect("hash");
        assert_eq!(digest, hex::encode(Sha256::digest(b"dvdforge")));
    }
}
