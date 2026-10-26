//! Fixed-length positional inputs shared by portable filesystem readers.
//!
//! Implementors promise that length and bytes remain stable for the lifetime of
//! all readers. A read-only operating-system handle alone does not establish
//! this promise. Reads have no shared cursor; concurrent callers can use separate
//! cursors. Implementors may return short reads and ordinary I/O errors (including
//! `Interrupted` for cancellation). EOF returns zero, including offsets beyond
//! the length; readers needing a complete range use [`ReadAt::read_exact_at`].

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

/// An immutable, fixed-length source with source-relative byte offsets.
pub trait ReadAt {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Read at most `buf.len()` bytes without changing any cursor.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;
    /// Read a complete bounded range; crossing EOF or arithmetic overflow fails.
    fn read_exact_at(&self, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
        let end = offset
            .checked_add(buf.len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "source range overflow"))?;
        if end > self.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "source range exceeds length",
            ));
        }
        while !buf.is_empty() {
            match self.read_at(offset, buf) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "short positional read",
                    ));
                }
                Ok(n) if n <= buf.len() => {
                    offset += n as u64;
                    buf = &mut buf[n..];
                }
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "source returned excessive read length",
                    ));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

impl<T: ReadAt + ?Sized> ReadAt for &T {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        (**self).read_at(offset, buf)
    }
}
impl<T: ReadAt + ?Sized> ReadAt for Arc<T> {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        (**self).read_at(offset, buf)
    }
}

/// Borrowed slice adapter; no copy of the image is made.
#[derive(Clone, Copy, Debug)]
pub struct SliceSource<'a> {
    bytes: &'a [u8],
}
impl<'a> SliceSource<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}
impl ReadAt for SliceSource<'_> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len() {
            return Ok(0);
        }
        let start = usize::try_from(offset).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "offset exceeds address space")
        })?;
        let count = buf.len().min(self.bytes.len() - start);
        buf[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

/// A checked region. Every exposed offset is relative to the region, including
/// offsets passed to ISO readers through [`SourceCursor`].
#[derive(Clone, Debug)]
pub struct BoundedSource<S> {
    source: S,
    start: u64,
    length: u64,
}
impl<S: ReadAt> BoundedSource<S> {
    pub fn new(source: S, start: u64, length: u64) -> io::Result<Self> {
        if start
            .checked_add(length)
            .is_none_or(|end| end > source.len())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "region exceeds source",
            ));
        }
        Ok(Self {
            source,
            start,
            length,
        })
    }
    pub fn into_inner(self) -> S {
        self.source
    }
}
impl<S: ReadAt> ReadAt for BoundedSource<S> {
    fn len(&self) -> u64 {
        self.length
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.length {
            return Ok(0);
        }
        let count = (self.length - offset).min(buf.len() as u64) as usize;
        let n = self
            .source
            .read_at(self.start + offset, &mut buf[..count])?;
        if n > count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "source returned excessive read length",
            ));
        }
        Ok(n)
    }
}

/// An independent checked `Read + Seek` cursor for ISO and other seek readers.
#[derive(Clone, Debug)]
pub struct SourceCursor<S> {
    source: S,
    position: u64,
}
impl<S: ReadAt> SourceCursor<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            position: 0,
        }
    }
    pub fn source(&self) -> &S {
        &self.source
    }
    pub fn into_inner(self) -> S {
        self.source
    }
}
impl<S: ReadAt> Read for SourceCursor<S> {
    fn read_exact(&mut self, mut buf: &mut [u8]) -> io::Result<()> {
        // Preserve Interrupted as cancellation instead of the standard Read
        // implementation retrying a permanently cancelled source indefinitely.
        while !buf.is_empty() {
            let count = self.read(buf)?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short positional cursor read",
                ));
            }
            buf = &mut buf[count..];
        }
        Ok(())
    }
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.source.len() {
            return Ok(0);
        }
        let count = (self.source.len() - self.position).min(buf.len() as u64) as usize;
        let n = self.source.read_at(self.position, &mut buf[..count])?;
        if n > count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "source returned excessive read length",
            ));
        }
        self.position += n as u64;
        Ok(n)
    }
}
impl<S: ReadAt> Seek for SourceCursor<S> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(value) => Some(value),
            SeekFrom::End(delta) => self.source.len().checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        }
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek overflow or negative position",
            )
        })?;
        self.position = next;
        Ok(next)
    }
}
