//! ISO9660 image reading and UDF 1.02 image creation with ISO9660 boot discovery.
//! All source files live in UDF; the ISO 9660 root is intentionally empty.
//! Descriptor layouts follow ECMA-167 (2nd edition), parts 3 and 4.
//! https://dev.ecma-international.org/wp-content/uploads/ECMA-167_2nd_edition_december_1994.pdf
//! El Torito catalog: https://read.seas.harvard.edu/~kohler/class/04f-aos/ref/hardware/boot-cdrom.pdf
//! Names are limited to 127 UTF-16 units; files to 234 short extents (~234 GiB).
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
const BLOCK: u64 = 2048;
const PART: u32 = 320;
const MAX_EXTENT: u64 = 0x3ffff800;
#[derive(Debug)]
struct Node {
    path: PathBuf,
    content: Option<crate::tree_source::DeferredContent>,
    name: Vec<u8>,
    parent: usize,
    children: Vec<usize>,
    directory: bool,
    size: u64,
    data: u32,
}
fn put16(b: &mut [u8], p: usize, v: u16) {
    b[p..p + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], p: usize, v: u32) {
    b[p..p + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], p: usize, v: u64) {
    b[p..p + 8].copy_from_slice(&v.to_le_bytes());
}
fn blocks(n: u64) -> Result<u64> {
    Ok(n.checked_add(BLOCK - 1).context("extent overflow")? / BLOCK)
}
fn crc(b: &[u8]) -> u16 {
    let mut c = 0u16;
    for x in b {
        c ^= u16::from(*x) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 {
                (c << 1) ^ 0x1021
            } else {
                c << 1
            };
        }
    }
    c
}
fn tag(b: &mut [u8], id: u16, loc: u32, len: usize) {
    put16(b, 0, id);
    put16(b, 2, 2);
    put16(b, 6, 1);
    put16(b, 8, crc(&b[16..len]));
    put16(b, 10, (len - 16) as u16);
    put32(b, 12, loc);
    b[4] = b[..16]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |s, (_, v)| s.wrapping_add(*v));
}
fn reg(b: &mut [u8], p: usize, s: &[u8], udf: bool) {
    b[p + 1..p + 1 + s.len()].copy_from_slice(s);
    if udf {
        put16(b, p + 24, 0x102);
    }
}
fn chars(b: &mut [u8], p: usize) {
    b[p + 1..p + 24].copy_from_slice(b"OSTA Compressed Unicode");
}
fn dstring(b: &mut [u8], p: usize, len: usize, s: &[u8]) {
    b[p] = 8;
    b[p + 1..p + 1 + s.len()].copy_from_slice(s);
    b[p + len - 1] = (s.len() + 1) as u8;
}
fn timestamp(b: &mut [u8], p: usize) {
    put16(b, p, 0x1000);
    put16(b, p + 2, 2026);
    b[p + 4] = 1;
    b[p + 5] = 1;
}
fn long_ad(b: &mut [u8], p: usize, len: u32, loc: u32) {
    put32(b, p, len);
    put32(b, p + 4, loc);
}
fn name(path: &Path) -> Result<Vec<u8>> {
    let s = path
        .file_name()
        .and_then(|s| s.to_str())
        .context("filename is not valid Unicode")?;
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut n = vec![16];
    for u in units {
        n.extend(u.to_be_bytes());
    }
    ensure!(
        n.len() <= 255,
        "UDF identifier exceeds 255 bytes: {}",
        path.display()
    );
    Ok(n)
}
fn scan(root: &Path, checkpoint: &mut impl FnMut() -> Result<()>) -> Result<Vec<Node>> {
    let mut ns = vec![Node {
        path: root.to_owned(),
        content: None,
        name: vec![],
        parent: 0,
        children: vec![],
        directory: true,
        size: 0,
        data: 0,
    }];
    let mut i = 0;
    while i < ns.len() {
        checkpoint()?;
        if ns[i].directory {
            let mut paths = fs::read_dir(&ns[i].path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            paths.sort();
            for path in paths {
                checkpoint()?;
                let m = fs::symlink_metadata(&path)?;
                ensure!(
                    m.is_file() || m.is_dir(),
                    "unsupported symlink or special file: {}",
                    path.display()
                );
                let idx = ns.len();
                let n = name(&path)?;
                ns.push(Node {
                    content: if m.is_file() {
                        Some(crate::tree_source::host_file_content(&path)?)
                    } else {
                        None
                    },
                    path,
                    name: n,
                    parent: i,
                    children: vec![],
                    directory: m.is_dir(),
                    size: if m.is_file() { m.len() } else { 0 },
                    data: 0,
                });
                ns[i].children.push(idx);
            }
        }
        i += 1;
    }
    Ok(ns)
}
fn fid(
    node: usize,
    n: &[u8],
    directory: bool,
    parent: bool,
    loc: u32,
    implementation: usize,
) -> Vec<u8> {
    let len = (38 + implementation + n.len() + 3) & !3;
    let mut b = vec![0; len];
    put16(&mut b, 16, 1);
    b[18] = if directory { 2 } else { 0 } | if parent { 8 } else { 0 };
    b[19] = n.len() as u8;
    long_ad(&mut b, 20, 2048, node as u32 + 1);
    put16(&mut b, 36, implementation as u16);
    if implementation != 0 {
        reg(&mut b, 38, b"*windows-uup", false);
    }
    b[38 + implementation..38 + implementation + n.len()].copy_from_slice(n);
    tag(&mut b, 257, loc, len);
    b
}
fn directory(ns: &[Node], i: usize) -> Vec<u8> {
    let mut b = vec![];
    let n = &ns[i];
    let records = std::iter::once((n.parent, &[][..], true, true)).chain(
        n.children
            .iter()
            .map(|j| (*j, ns[*j].name.as_slice(), ns[*j].directory, false)),
    );
    for (j, name, dir, parent) in records {
        let len = (38 + name.len() + 3) & !3;
        // FIDs form a contiguous stream and may cross a block boundary.
        // Extend this FID rather than insert an invalid zero-tag gap, and
        // keep the next 16-byte descriptor tag entirely within its block.
        let remaining = (2048 - (b.len() + len) % 2048) % 2048;
        let implementation = if remaining > 0 && remaining < 16 {
            remaining + 32
        } else {
            0
        };
        let loc = n.data + (b.len() / 2048) as u32;
        b.extend(fid(j, name, dir, parent, loc, implementation));
    }
    b
}
fn entry(n: &Node, i: usize) -> Result<[u8; 2048]> {
    let mut b = [0; 2048];
    put16(&mut b, 20, 4);
    put16(&mut b, 24, 1);
    b[27] = if n.directory { 4 } else { 5 };
    put32(&mut b, 36, u32::MAX);
    put32(&mut b, 40, u32::MAX);
    put32(&mut b, 44, 0x14a5);
    put16(&mut b, 48, 1);
    put64(&mut b, 56, n.size);
    put64(&mut b, 64, blocks(n.size)?);
    for p in [72, 84, 96] {
        timestamp(&mut b, p);
    }
    put32(&mut b, 108, 1);
    reg(&mut b, 128, b"*windows-uup", false);
    put64(&mut b, 160, i as u64 + 16);
    let mut remain = n.size;
    let mut pos = n.data;
    let mut p = 176;
    while remain > 0 {
        ensure!(
            p + 8 <= 2048,
            "file needs allocation continuation: maximum supported file is about 234 GiB"
        );
        let len = remain.min(MAX_EXTENT);
        put32(&mut b, p, len as u32);
        put32(&mut b, p + 4, pos);
        pos = pos
            .checked_add(u32::try_from(blocks(len)?)?)
            .context("extent overflow")?;
        remain -= len;
        p += 8;
    }
    put32(&mut b, 172, (p - 176) as u32);
    tag(&mut b, 261, i as u32 + 1, p);
    Ok(b)
}
fn descriptor(id: u16, loc: u32, part_len: u32, files: u32, dirs: u32) -> [u8; 2048] {
    let mut b = [0; 2048];
    let len = match id {
        1 => {
            put32(&mut b, 16, 1);
            dstring(&mut b, 24, 32, b"WINDOWS_UUP");
            put16(&mut b, 56, 1);
            put16(&mut b, 58, 1);
            put16(&mut b, 60, 2);
            put16(&mut b, 62, 3);
            put32(&mut b, 64, 1);
            put32(&mut b, 68, 1);
            dstring(&mut b, 72, 128, b"WINDOWS_UUP_2026");
            chars(&mut b, 200);
            chars(&mut b, 264);
            timestamp(&mut b, 376);
            reg(&mut b, 388, b"*windows-uup", false);
            512
        }
        4 => {
            put32(&mut b, 16, 2);
            reg(&mut b, 20, b"*UDF LV Info", true);
            chars(&mut b, 52);
            dstring(&mut b, 116, 128, b"WINDOWS_UUP");
            reg(&mut b, 352, b"*windows-uup", false);
            512
        }
        5 => {
            put32(&mut b, 16, 3);
            put16(&mut b, 20, 1);
            reg(&mut b, 24, b"+NSR02", false);
            put32(&mut b, 184, 1);
            put32(&mut b, 188, PART);
            put32(&mut b, 192, part_len);
            reg(&mut b, 196, b"*windows-uup", false);
            512
        }
        6 => {
            put32(&mut b, 16, 4);
            chars(&mut b, 20);
            dstring(&mut b, 84, 128, b"WINDOWS_UUP");
            put32(&mut b, 212, 2048);
            reg(&mut b, 216, b"*OSTA UDF Compliant", true);
            long_ad(&mut b, 248, 2048, 0);
            put32(&mut b, 264, 6);
            put32(&mut b, 268, 1);
            reg(&mut b, 272, b"*windows-uup", false);
            put32(&mut b, 432, 4096);
            put32(&mut b, 436, 272);
            b[440] = 1;
            b[441] = 6;
            put16(&mut b, 442, 1);
            446
        }
        7 => {
            put32(&mut b, 16, 5);
            24
        }
        9 => {
            timestamp(&mut b, 16);
            put32(&mut b, 28, 1);
            put64(&mut b, 40, u64::from(files) + u64::from(dirs) + 16);
            put32(&mut b, 72, 1);
            put32(&mut b, 76, 46);
            put32(&mut b, 80, 0);
            put32(&mut b, 84, part_len);
            reg(&mut b, 88, b"*windows-uup", false);
            put32(&mut b, 120, files);
            put32(&mut b, 124, dirs);
            put16(&mut b, 128, 0x102);
            put16(&mut b, 130, 0x102);
            put16(&mut b, 132, 0x102);
            134
        }
        256 => {
            timestamp(&mut b, 16);
            put16(&mut b, 28, 3);
            put16(&mut b, 30, 3);
            put32(&mut b, 32, 1);
            put32(&mut b, 36, 1);
            chars(&mut b, 48);
            dstring(&mut b, 112, 128, b"WINDOWS_UUP");
            chars(&mut b, 240);
            dstring(&mut b, 304, 32, b"WINDOWS_UUP");
            long_ad(&mut b, 400, 2048, 1);
            reg(&mut b, 416, b"*OSTA UDF Compliant", true);
            512
        }
        _ => 512,
    };
    tag(&mut b, id, loc, len);
    b
}
fn sector(f: &mut File, loc: u32, b: &[u8]) -> Result<()> {
    f.seek(SeekFrom::Start(u64::from(loc) * BLOCK))?;
    f.write_all(b)?;
    Ok(())
}
fn anchor(loc: u32, reserve: u32) -> [u8; 2048] {
    let mut b = [0; 2048];
    put32(&mut b, 16, 6 * 2048);
    put32(&mut b, 20, 257);
    put32(&mut b, 24, 6 * 2048);
    put32(&mut b, 28, reserve);
    tag(&mut b, 2, loc, 512);
    b
}
fn both32(b: &mut [u8], p: usize, v: u32) {
    put32(b, p, v);
    b[p + 4..p + 8].copy_from_slice(&v.to_be_bytes());
}
fn both16(b: &mut [u8], p: usize, v: u16) {
    put16(b, p, v);
    b[p + 2..p + 4].copy_from_slice(&v.to_be_bytes());
}
fn iso_record(id: u8) -> [u8; 34] {
    let mut b = [0; 34];
    b[0] = 34;
    both32(&mut b, 2, 281);
    both32(&mut b, 10, 2048);
    b[18] = 126;
    b[19] = 1;
    b[20] = 1;
    b[25] = 2;
    both16(&mut b, 28, 1);
    b[32] = 1;
    b[33] = id;
    b
}
fn boot_entry(b: &mut [u8], p: usize, n: &Node) -> Result<()> {
    ensure!(
        n.size > 0 && n.size.is_multiple_of(512),
        "boot images must be nonempty and 512-byte aligned"
    );
    let count = u16::try_from(n.size / 512)
        .context("boot image exceeds El Torito 16-bit load-count capacity")?;
    b[p] = 0x88;
    put16(b, p + 6, count);
    put32(b, p + 8, PART + n.data);
    Ok(())
}
fn validate_descriptor(f: &mut File, physical: u32, logical: u32, id: u16) -> Result<()> {
    let mut b = [0u8; 2048];
    f.seek(SeekFrom::Start(u64::from(physical) * BLOCK))?;
    f.read_exact(&mut b)?;
    ensure!(
        u16::from_le_bytes([b[0], b[1]]) == id,
        "descriptor type mismatch at sector {physical}"
    );
    ensure!(
        u32::from_le_bytes(b[12..16].try_into()?) == logical,
        "descriptor location mismatch"
    );
    let len = usize::from(u16::from_le_bytes([b[10], b[11]]));
    ensure!(len <= 2032, "descriptor CRC length exceeds sector");
    ensure!(
        crc(&b[16..16 + len]) == u16::from_le_bytes([b[8], b[9]]),
        "descriptor CRC mismatch"
    );
    let checksum = b[..16]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |s, (_, v)| s.wrapping_add(*v));
    ensure!(checksum == b[4], "descriptor checksum mismatch");
    Ok(())
}
/// Write deterministic UDF media, refusing overwrite and publishing only a closed,
/// hashed file. BIOS/EFI VM and Windows mounting validation are still required.
pub fn write_iso(source: &Path, output: &Path) -> Result<()> {
    write_iso_with_hash(source, output).map(|_| ())
}
/// Write and publish optical media, returning its lower-case SHA-256 hex digest.
pub fn write_iso_with_hash(source: &Path, output: &Path) -> Result<String> {
    write_iso_with_cancel(source, output, || Ok(()))
}
/// Write optical media with cooperative cancellation during scanning, layout,
/// payload copying, verification, hashing and immediately before publication.
/// Returning an error from `checkpoint` removes the temporary image and leaves
/// the output unpublished. Each payload copy reads at most 64 KiB per check.
pub fn write_iso_with_cancel(
    source: &Path,
    output: &Path,
    mut checkpoint: impl FnMut() -> Result<()>,
) -> Result<String> {
    checkpoint()?;
    let source = source.canonicalize()?;
    ensure!(source.is_dir(), "source must be a directory");
    ensure!(!output.exists(), "output already exists");
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    ensure!(
        !parent.starts_with(&source),
        "output must be outside source tree"
    );
    let mut ns = scan(&source, &mut checkpoint)?;
    let mut cursor = u32::try_from(ns.len() + 1)?;
    for i in 0..ns.len() {
        checkpoint()?;
        if ns[i].directory {
            ns[i].data = cursor;
            ns[i].size = directory(&ns, i).len() as u64;
            cursor = cursor
                .checked_add(u32::try_from(blocks(ns[i].size)?)?)
                .context("layout exceeds UDF limits")?;
        }
    }
    // Optical firmware may impose early-disc boot access limits. Keep both
    // catalog images ahead of large install payloads regardless of tree order.
    let mut file_order: Vec<usize> = (0..ns.len()).filter(|i| !ns[*i].directory).collect();
    file_order.sort_by_key(|i| {
        let relative = ns[*i]
            .path
            .strip_prefix(&source)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .replace('\\', "/")
            .to_ascii_lowercase();
        match relative.as_str() {
            "boot/etfsboot.com" | "efi/microsoft/boot/efisys.bin" => 0,
            "bootmgr" | "bootmgr.efi" => 1,
            _ => 2,
        }
    });
    for i in file_order {
        checkpoint()?;
        let n = &mut ns[i];
        if !n.directory {
            n.data = cursor;
            cursor = cursor
                .checked_add(u32::try_from(blocks(n.size)?)?)
                .context("layout exceeds UDF limits")?;
        }
    }
    for (i, n) in ns.iter().enumerate() {
        checkpoint()?;
        entry(n, i)?;
    }
    let bios = ns
        .iter()
        .find(|n| n.path.strip_prefix(&source).ok() == Some(Path::new("boot/etfsboot.com")))
        .context("missing boot/etfsboot.com")?;
    let efi = ns
        .iter()
        .find(|n| {
            n.path.strip_prefix(&source).ok() == Some(Path::new("efi/microsoft/boot/efisys.bin"))
        })
        .context("missing efi/microsoft/boot/efisys.bin")?;
    let mut catalog = [0; 2048];
    catalog[0] = 1;
    catalog[30] = 0x55;
    catalog[31] = 0xaa;
    let sum = catalog[..32].chunks_exact(2).fold(0u16, |s, v| {
        s.wrapping_add(u16::from_le_bytes([v[0], v[1]]))
    });
    put16(&mut catalog, 28, 0u16.wrapping_sub(sum));
    boot_entry(&mut catalog, 32, bios)?;
    catalog[64] = 0x91;
    catalog[65] = 0xef;
    put16(&mut catalog, 66, 1);
    boot_entry(&mut catalog, 96, efi)?;
    let reserve = PART
        .checked_add(cursor)
        .context("volume exceeds UDF limits")?;
    let total = reserve
        .checked_add(273)
        .context("volume exceeds UDF limits")?;
    let (file, temp) = tempfile::NamedTempFile::new_in(&parent)?.into_parts();
    (|| -> Result<String> {
        let mut f = file;
        f.set_len(u64::from(total) * BLOCK)?;
        let mut pvd = [0; 2048];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[6] = 1;
        pvd[8..40].fill(b' ');
        pvd[40..72].fill(b' ');
        pvd[40..51].copy_from_slice(b"WINDOWS_UUP");
        both32(&mut pvd, 80, total);
        both16(&mut pvd, 120, 1);
        both16(&mut pvd, 124, 1);
        both16(&mut pvd, 128, 2048);
        both32(&mut pvd, 132, 10);
        put32(&mut pvd, 140, 279);
        pvd[148..152].copy_from_slice(&280u32.to_be_bytes());
        pvd[156..190].copy_from_slice(&iso_record(0));
        for offset in [813, 830, 847, 864] {
            pvd[offset..offset + 16].fill(b'0');
        }
        pvd[881] = 1;
        sector(&mut f, 16, &pvd)?;
        let mut boot = [0; 2048];
        boot[1..6].copy_from_slice(b"CD001");
        boot[6] = 1;
        boot[7..30].copy_from_slice(b"EL TORITO SPECIFICATION");
        put32(&mut boot, 71, 278);
        sector(&mut f, 17, &boot)?;
        let mut term = [0; 2048];
        term[0] = 255;
        term[1..6].copy_from_slice(b"CD001");
        term[6] = 1;
        sector(&mut f, 18, &term)?;
        for (loc, id) in [(19, b"BEA01"), (20, b"NSR02"), (21, b"TEA01")] {
            let mut b = [0; 2048];
            b[1..6].copy_from_slice(id);
            b[6] = 1;
            sector(&mut f, loc, &b)?;
        }
        sector(&mut f, 278, &catalog)?;
        for (loc, big) in [(279, false), (280, true)] {
            let mut b = [0; 2048];
            b[0] = 1;
            if big {
                b[2..6].copy_from_slice(&281u32.to_be_bytes());
                b[6..8].copy_from_slice(&1u16.to_be_bytes());
            } else {
                put32(&mut b, 2, 281);
                put16(&mut b, 6, 1);
            }
            sector(&mut f, loc, &b)?;
        }
        let mut root = [0; 2048];
        root[..34].copy_from_slice(&iso_record(0));
        root[34..68].copy_from_slice(&iso_record(1));
        sector(&mut f, 281, &root)?;
        let files = ns.iter().filter(|n| !n.directory).count() as u32;
        let dirs = ns.len() as u32 - files;
        for (offset, id) in [1, 4, 5, 6, 7, 8].into_iter().enumerate() {
            for start in [257, reserve] {
                sector(
                    &mut f,
                    start + offset as u32,
                    &descriptor(id, start + offset as u32, cursor, files, dirs),
                )?;
            }
        }
        sector(&mut f, 272, &descriptor(9, 272, cursor, files, dirs))?;
        sector(&mut f, 273, &descriptor(8, 273, cursor, files, dirs))?;
        for loc in [256, total - 257, total - 1] {
            sector(&mut f, loc, &anchor(loc, reserve))?;
        }
        sector(&mut f, PART, &descriptor(256, 0, cursor, files, dirs))?;
        for (i, n) in ns.iter().enumerate() {
            checkpoint()?;
            sector(&mut f, PART + i as u32 + 1, &entry(n, i)?)?;
            if n.directory {
                sector(&mut f, PART + n.data, &directory(&ns, i))?;
            } else {
                f.seek(SeekFrom::Start(u64::from(PART + n.data) * BLOCK))?;
                let input = n
                    .content
                    .as_ref()
                    .context("missing deferred source content")?;
                let mut copied = 0;
                let mut buffer = [0u8; 65536];
                while copied < n.size {
                    checkpoint()?;
                    let count = (n.size - copied).min(buffer.len() as u64) as usize;
                    input.read_exact_at(copied, &mut buffer[..count])?;
                    let read = count;
                    if read == 0 {
                        break;
                    }
                    f.write_all(&buffer[..read])?;
                    copied += read as u64;
                }
                input.validate()?;
                ensure!(
                    copied == n.size,
                    "source changed during ISO write: {}",
                    n.path.display()
                );
            }
        }
        f.sync_all()?;
        for loc in [256, total - 257, total - 1] {
            validate_descriptor(&mut f, loc, loc, 2)?;
        }
        for start in [257, reserve] {
            for (offset, id) in [1, 4, 5, 6, 7, 8].into_iter().enumerate() {
                validate_descriptor(&mut f, start + offset as u32, start + offset as u32, id)?;
            }
        }
        validate_descriptor(&mut f, PART, 0, 256)?;
        for i in 0..ns.len() {
            checkpoint()?;
            validate_descriptor(&mut f, PART + i as u32 + 1, i as u32 + 1, 261)?;
        }

        f.seek(SeekFrom::Start(0))?;
        let mut digest = Sha256::new();
        let mut buf = [0u8; 65536];
        loop {
            checkpoint()?;
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            digest.update(&buf[..n]);
        }
        let hash = hex::encode(digest.finalize());
        drop(f);
        checkpoint()?;
        for node in &ns {
            checkpoint()?;
            if let Some(content) = &node.content {
                content.validate()?;
            }
        }
        fs::hard_link(&temp, output)
            .context("publish ISO without overwriting an existing output")?;

        Ok(hash)
    })()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_after_temporary_image_creation_never_publishes_or_leaks_image() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        fs::create_dir_all(source.join("boot")).unwrap();
        fs::create_dir_all(source.join("efi/microsoft/boot")).unwrap();
        fs::write(source.join("boot/etfsboot.com"), [1; 4096]).unwrap();
        fs::write(source.join("efi/microsoft/boot/efisys.bin"), [2; 4096]).unwrap();
        File::create(source.join("payload.bin"))
            .unwrap()
            .set_len(8 * 1024 * 1024)
            .unwrap();
        let output = tmp.path().join("cancelled.iso");
        let mut checkpoints_with_temp = 0;
        let error = write_iso_with_cancel(&source, &output, || {
            if fs::read_dir(tmp.path())?.any(|entry| entry.is_ok_and(|e| e.path().is_file())) {
                checkpoints_with_temp += 1;
                if checkpoints_with_temp == 8 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "cancelled by caller",
                    )
                    .into());
                }
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::Interrupted
        );
        assert!(!output.exists());
        let remaining: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(remaining, vec![source]);
    }
    #[test]
    fn writes_reproducible_media_and_preserves_existing_output() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        fs::create_dir_all(source.join("boot")).unwrap();
        fs::create_dir_all(source.join("efi/microsoft/boot")).unwrap();
        fs::write(source.join("boot/etfsboot.com"), [1; 4096]).unwrap();
        fs::write(source.join("efi/microsoft/boot/efisys.bin"), [2; 4096]).unwrap();
        fs::write(source.join("日本語.txt"), b"Unicode").unwrap();
        fs::write(source.join("empty"), []).unwrap();
        let first = tmp.path().join("first.iso");
        let second = tmp.path().join("second.iso");
        let hash = write_iso_with_hash(&source, &first).unwrap();
        write_iso(&source, &second).unwrap();
        let bytes = fs::read(&first).unwrap();
        assert_eq!(hash, hex::encode(Sha256::digest(&bytes)));
        assert_eq!(bytes, fs::read(&second).unwrap());
        assert!(write_iso(&source, &first).is_err());
        assert_eq!(bytes, fs::read(&first).unwrap());
        let anchor = &bytes[256 * 2048..257 * 2048];
        let reserve = u32::from_le_bytes(anchor[28..32].try_into().unwrap()) as usize;
        assert_eq!(
            u16::from_le_bytes(
                bytes[reserve * 2048..reserve * 2048 + 2]
                    .try_into()
                    .unwrap()
            ),
            1
        );
        let catalog = &bytes[278 * 2048..279 * 2048];
        assert_eq!(
            catalog[..32].chunks_exact(2).fold(0u16, |s, v| s
                .wrapping_add(u16::from_le_bytes([v[0], v[1]]))),
            0
        );
        assert_eq!((catalog[32], catalog[65], catalog[96]), (0x88, 0xef, 0x88));
    }
    #[test]
    fn rejects_output_inside_source_before_creating_partial_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(write_iso(tmp.path(), &tmp.path().join("bad.iso")).is_err());
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }
    #[test]
    fn directory_records_are_contiguous_across_sectors() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..200 {
            fs::write(root.path().join(format!("file-{i:03}.txt")), []).unwrap();
        }
        let ns = scan(root.path(), &mut || Ok(())).unwrap();
        let bytes = directory(&ns, 0);
        let mut offset = 0;
        let mut count = 0;
        while offset < bytes.len() {
            let b = &bytes[offset..];
            assert!(offset % 2048 <= 2032);
            assert_eq!(u16::from_le_bytes([b[0], b[1]]), 257);
            let implementation = usize::from(u16::from_le_bytes([b[36], b[37]]));
            assert!(implementation == 0 || implementation >= 32);
            let len = (38 + implementation + usize::from(b[19]) + 3) & !3;
            assert_eq!(crc(&b[16..len]), u16::from_le_bytes([b[8], b[9]]));
            offset += len;
            count += 1;
        }
        assert_eq!(count, 201);
    }
    #[test]
    fn crc_matches_ecma_polynomial() {
        assert_eq!(crc(b"123456789"), 0x31c3);
    }
    #[test]
    fn block_rounding_rejects_overflow() {
        assert!(blocks(u64::MAX).is_err());
    }
    #[test]
    fn allocation_supports_larger_than_four_gib() {
        let n = Node {
            path: PathBuf::new(),
            content: None,
            name: vec![],
            parent: 0,
            children: vec![],
            directory: false,
            size: (1u64 << 32) + 2048,
            data: 8,
        };
        let b = entry(&n, 0).unwrap();
        assert_eq!(u64::from_le_bytes(b[56..64].try_into().unwrap()), n.size);
        assert_eq!(u32::from_le_bytes(b[172..176].try_into().unwrap()), 40);
    }
    #[test]
    fn tag_checksum_excludes_checksum_byte() {
        let b = anchor(256, 1000);
        assert_eq!(
            b[..16]
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != 4)
                .fold(0u8, |s, (_, v)| s.wrapping_add(*v)),
            b[4]
        );
    }
}
