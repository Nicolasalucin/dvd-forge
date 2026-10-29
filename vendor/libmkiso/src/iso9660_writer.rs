//! Deterministic ISO9660 levels 1–3, Joliet/Rock Ridge and hybrid boot authoring.
use crate::el_torito::{BootEntry, advanced_catalog, boot_descriptor};
use crate::iso_options::{FilenamePolicy, IsoLevel, IsoOptions};
use crate::iso9660::{Error, Result};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const BLOCK: usize = 2048;
#[derive(Clone)]
struct Node {
    path: PathBuf,
    content: Option<crate::tree_source::DeferredContent>,
    name: Vec<u8>,
    joliet_name: Vec<u8>,
    parent: usize,
    directory: bool,
    size: u64,
    sector: u32,
    children: Vec<usize>,
    original: String,
    unix: crate::rock_ridge::UnixMetadata,
    link: Option<String>,
    identity: Option<(u64, u64)>,
    rr_offset: u32,
    rr_length: u32,
}
fn unsupported(message: &str) -> Error {
    Error::Unsupported(message.into())
}
fn number32(target: &mut [u8], value: u32) {
    target[..4].copy_from_slice(&value.to_le_bytes());
    target[4..8].copy_from_slice(&value.to_be_bytes());
}
fn number16(target: &mut [u8], value: u16) {
    target[..2].copy_from_slice(&value.to_le_bytes());
    target[2..4].copy_from_slice(&value.to_be_bytes());
}
fn record(node: &Node, name: &[u8], options: &IsoOptions) -> Vec<u8> {
    let mut bytes = vec![0; 33 + name.len() + usize::from(name.len().is_multiple_of(2))];
    bytes[0] = bytes.len() as u8;
    number32(&mut bytes[2..10], node.sector);
    number32(&mut bytes[10..18], node.size as u32);
    bytes[18..25].copy_from_slice(&options.timestamp.directory_bytes());
    bytes[25] = if node.directory { 2 } else { 0 };
    number16(&mut bytes[28..32], 1);
    bytes[32] = name.len() as u8;
    bytes[33..33 + name.len()].copy_from_slice(name);
    bytes
}
fn directory(
    nodes: &[Node],
    index: usize,
    options: &IsoOptions,
    joliet: bool,
    rr_sector: u32,
) -> Result<Vec<u8>> {
    let node = &nodes[index];
    let mut bytes = Vec::new();
    let mut records = vec![
        record(node, &[0], options),
        record(&nodes[node.parent], &[1], options),
    ];
    if options.rock_ridge && !joliet {
        for (ordinal, item) in records.iter_mut().enumerate() {
            if index == 0 && ordinal == 0 {
                item.extend(crate::rock_ridge::writer::discovery());
            }
            let target = if ordinal == 0 {
                node
            } else {
                &nodes[node.parent]
            };
            item.extend(crate::rock_ridge::writer::attributes(
                &target.unix,
                options.timestamp.directory_bytes(),
                None,
                None,
            ));
            item[0] = item.len() as u8;
        }
    }
    for item in records {
        append_record(&mut bytes, &item, options.max_metadata_bytes)?;
    }
    for &i in &node.children {
        let child = &nodes[i];
        let cap = if child.directory || options.level != IsoLevel::Level3 {
            u64::from(u32::MAX)
        } else {
            u64::from(options.extent_bytes)
        };
        let mut offset = 0;
        loop {
            let amount = (child.size - offset).min(cap);
            let mut item = record(child, &child.name, options);
            number32(
                &mut item[2..10],
                child.sector + (offset / BLOCK as u64) as u32,
            );
            number32(&mut item[10..18], amount as u32);
            if offset + amount < child.size {
                item[25] |= 0x80;
            }
            if options.rock_ridge && !joliet {
                item.extend(crate::rock_ridge::writer::continuation(
                    rr_sector + child.rr_offset / BLOCK as u32,
                    child.rr_offset % BLOCK as u32,
                    child.rr_length,
                ));
                item[0] = item.len() as u8;
            }
            append_record(&mut bytes, &item, options.max_metadata_bytes)?;
            offset += amount;
            if offset == child.size {
                break;
            }
        }
    }
    if bytes.len().div_ceil(BLOCK).saturating_mul(BLOCK) > options.max_metadata_bytes {
        return Err(Error::ResourceLimit("ISO writer directory bytes"));
    }
    bytes.resize(bytes.len().div_ceil(BLOCK) * BLOCK, 0);
    Ok(bytes)
}
fn append_record(bytes: &mut Vec<u8>, item: &[u8], maximum: usize) -> Result<()> {
    if item.len() > 255 {
        return Err(unsupported("ISO directory record exceeds 255 bytes"));
    }
    let remaining = BLOCK - bytes.len() % BLOCK;
    let padding = if item.len() > remaining { remaining } else { 0 };
    let new = bytes
        .len()
        .checked_add(padding)
        .and_then(|value| value.checked_add(item.len()))
        .ok_or(Error::ResourceLimit("ISO writer directory bytes"))?;
    if new > maximum {
        return Err(Error::ResourceLimit("ISO writer directory bytes"));
    }
    bytes.resize(bytes.len() + padding, 0);
    bytes.extend(item);
    Ok(())
}
fn identifier(name: &str, directory: bool) -> Result<Vec<u8>> {
    let name = name.to_ascii_uppercase();
    let valid = |s: &str| {
        s.bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
    };
    if directory {
        if name.is_empty() || name.len() > 31 || !valid(&name) {
            return Err(unsupported(
                "directory names require 1–31 ASCII letters, digits or underscores",
            ));
        }
        return Ok(name.into_bytes());
    }
    let (base, extension) = name.split_once('.').unwrap_or((&name, ""));
    if base.is_empty() || !valid(base) || !valid(extension) || base.len() + extension.len() > 30 {
        return Err(unsupported(
            "file names require ASCII letters, digits, underscores and at most one dot (30 characters excluding the dot)",
        ));
    }
    Ok(format!("{base}.{extension};1").into_bytes())
}
fn store_attributes(
    pool: &mut Vec<u8>,
    bytes: &[u8],
    relocations: &mut Vec<usize>,
    maximum: usize,
) -> Result<(u32, u32)> {
    let mut groups = vec![Vec::new()];
    let mut offset = 0;
    while offset < bytes.len() {
        let length = usize::from(bytes[offset + 2]);
        if groups
            .last()
            .is_some_and(|group| group.len() + length > 2000)
        {
            groups.push(Vec::new());
        }
        groups
            .last_mut()
            .ok_or_else(|| unsupported("Rock Ridge attribute group"))?
            .extend_from_slice(&bytes[offset..offset + length]);
        offset += length;
    }
    let mut first = None;
    for (index, group) in groups.iter().enumerate() {
        let more = index + 1 < groups.len();
        let size = group.len() + if more { 28 } else { 0 };
        let padding = if pool.len() % BLOCK + size > BLOCK {
            BLOCK - pool.len() % BLOCK
        } else {
            0
        };
        if pool.len().saturating_add(padding).saturating_add(size) > maximum {
            return Err(Error::ResourceLimit("Rock Ridge writer metadata"));
        }
        pool.resize(pool.len() + padding, 0);
        first.get_or_insert((
            u32::try_from(pool.len())
                .map_err(|_| Error::ResourceLimit("Rock Ridge writer metadata"))?,
            size as u32,
        ));
        pool.extend(group);
        if more {
            let next_block = pool.len() / BLOCK + 1;
            let next_size = groups[index + 1].len() + if index + 2 < groups.len() { 28 } else { 0 };
            relocations.push(pool.len() + 4);
            pool.extend(crate::rock_ridge::writer::continuation(
                next_block as u32,
                0,
                next_size as u32,
            ));
            let end = next_block * BLOCK;
            if end > maximum {
                return Err(Error::ResourceLimit("Rock Ridge writer metadata"));
            }
            pool.resize(end, 0);
        }
    }
    first.ok_or_else(|| unsupported("empty Rock Ridge attributes"))
}
fn scan(
    nodes: &mut Vec<Node>,
    index: usize,
    depth: usize,
    path_length: usize,
    options: &IsoOptions,
) -> Result<()> {
    let mut children = Vec::new();
    for entry in fs::read_dir(&nodes[index].path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if !(kind.is_dir() || kind.is_file() || kind.is_symlink() && options.rock_ridge) {
            return Err(unsupported("symlinks and special source files"));
        }
        let source_name = entry.file_name();
        let source_name = source_name
            .to_str()
            .ok_or_else(|| unsupported("non-UTF8 source names"))?;
        let joliet_name = if options.joliet {
            joliet_identifier(source_name, options.joliet_max_name)?
        } else {
            Vec::new()
        };
        let name = if options.filename_policy == FilenamePolicy::Mangle {
            mangle_for_level(source_name, kind.is_dir(), options.level)
        } else {
            identifier_for_level(source_name, kind.is_dir(), options.level)?
        };
        if (kind.is_dir() && depth >= options.max_directory_depth)
            || path_length + name.len() > options.max_path_bytes
        {
            return Err(unsupported("ISO9660 directory depth or path length"));
        }
        let size = if kind.is_file() {
            let size = entry.metadata()?.len();
            if options.level != IsoLevel::Level3 && size > u64::from(u32::MAX) {
                return Err(unsupported("files larger than 4 GiB require ISO level 3"));
            }
            if size.div_ceil(u64::from(options.extent_bytes))
                > (options.max_metadata_bytes / 34) as u64
                || size.div_ceil(BLOCK as u64) > u64::from(u32::MAX)
            {
                return Err(Error::ResourceLimit("ISO writer file sections"));
            }
            size
        } else {
            0
        };
        children.push(Node {
            path: entry.path(),
            content: if kind.is_file() {
                Some(crate::tree_source::host_file_content(entry.path())?)
            } else {
                None
            },
            name,
            joliet_name,
            parent: index,
            directory: kind.is_dir(),
            size,
            sector: 0,
            children: Vec::new(),
            original: source_name.into(),
            unix: source_metadata(
                &fs::symlink_metadata(entry.path())?,
                kind.is_dir(),
                kind.is_symlink(),
                options,
            ),
            link: if kind.is_symlink() {
                Some(
                    fs::read_link(entry.path())?
                        .to_str()
                        .ok_or_else(|| unsupported("non-UTF8 symbolic link"))?
                        .into(),
                )
            } else {
                None
            },
            identity: file_identity(&entry.metadata()?, kind.is_file(), options),
            rr_offset: 0,
            rr_length: 0,
        });
        if nodes.len() + children.len() > options.max_entries {
            return Err(Error::ResourceLimit("ISO writer entries"));
        }
    }
    children.sort_by(|a, b| a.path.cmp(&b.path));
    let mut used = HashSet::new();
    for (ordinal, node) in children.iter_mut().enumerate() {
        assign_primary_name(node, ordinal, &mut used, options)?;
    }
    children.sort_by(|a, b| a.name.cmp(&b.name));
    for node in children {
        if path_length + node.name.len() > options.max_path_bytes {
            return Err(unsupported("ISO9660 path length after aliasing"));
        }
        let child = nodes.len();
        let directory = node.directory;
        let length = path_length + node.name.len() + 1;
        nodes.push(node);
        nodes[index].children.push(child);
        if directory {
            scan(nodes, child, depth + 1, length, options)?;
        }
    }
    Ok(())
}
// Host and deferred inventories use the same display-name collision rules.
fn assign_primary_name(
    node: &mut Node,
    ordinal: usize,
    used: &mut HashSet<Vec<u8>>,
    options: &IsoOptions,
) -> Result<()> {
    if used.insert(display_identifier(node)) {
        return Ok(());
    }
    if options.filename_policy != FilenamePolicy::Mangle {
        return Err(unsupported("source names collide after ASCII uppercasing"));
    }
    let mut suffix = 0usize;
    loop {
        let alias = if node.directory {
            format!("D{ordinal:X}_{suffix:X}")
        } else {
            format!("F{ordinal:X}_{suffix:X}")
        };
        node.name = identifier_for_level(&alias, node.directory, options.level)?;
        if used.insert(display_identifier(node)) {
            return Ok(());
        }
        suffix += 1;
    }
}
fn identifier_for_level(name: &str, directory: bool, level: IsoLevel) -> Result<Vec<u8>> {
    let bytes = identifier(name, directory)?;
    if level == IsoLevel::Level1 {
        if directory && bytes.len() > 8 {
            return Err(unsupported(
                "level 1 directories require at most eight characters",
            ));
        }
        if !directory {
            let plain = bytes.strip_suffix(b";1").unwrap_or(&bytes);
            let split = plain.iter().position(|&b| b == b'.').unwrap_or(plain.len());
            if split > 8 || plain.len().saturating_sub(split + 1) > 3 {
                return Err(unsupported("level 1 files require 8.3 identifiers"));
            }
        }
    }
    Ok(bytes)
}
fn mangle_for_level(name: &str, directory: bool, level: IsoLevel) -> Vec<u8> {
    let bytes = mangle_identifier(name, directory);
    if level != IsoLevel::Level1 {
        return bytes;
    }
    if directory {
        return bytes.into_iter().take(8).collect();
    }
    let plain = bytes.strip_suffix(b";1").unwrap_or(&bytes);
    let split = plain.iter().position(|&b| b == b'.').unwrap_or(plain.len());
    let mut result = plain[..split.min(8)].to_vec();
    result.push(b'.');
    if split < plain.len() {
        result.extend(plain[split + 1..].iter().take(3));
    }
    result.extend(b";1");
    result
}
fn source_metadata(
    metadata: &fs::Metadata,
    directory: bool,
    link: bool,
    options: &IsoOptions,
) -> crate::rock_ridge::UnixMetadata {
    let policy = &options.unix_metadata;
    let kind = if directory {
        0o040000
    } else if link {
        0o120000
    } else {
        0o100000
    };
    let value = crate::rock_ridge::UnixMetadata {
        mode: kind
            | if directory {
                policy.directory_mode
            } else if link {
                0o777
            } else {
                policy.file_mode
            },
        links: 1,
        uid: policy.uid,
        gid: policy.gid,
        serial: None,
        device: None,
        timestamps: Vec::new(),
    };
    #[cfg(unix)]
    if policy.preserve {
        use std::os::unix::fs::MetadataExt;
        return crate::rock_ridge::UnixMetadata {
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            ..value
        };
    }
    #[cfg(not(unix))]
    let _ = metadata;
    value
}
fn file_identity(metadata: &fs::Metadata, file: bool, options: &IsoOptions) -> Option<(u64, u64)> {
    #[cfg(unix)]
    if file && options.rock_ridge {
        use std::os::unix::fs::MetadataExt;
        return Some((metadata.dev(), metadata.ino()));
    }
    let _ = (metadata, file, options);
    None
}
fn display_identifier(node: &Node) -> Vec<u8> {
    if node.directory {
        return node.name.clone();
    }
    let name = node.name.strip_suffix(b";1").unwrap_or(&node.name);
    name.strip_suffix(b".").unwrap_or(name).to_vec()
}
fn path_table(nodes: &[Node], dirs: &[usize], big: bool) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for &i in dirs {
        let node = &nodes[i];
        let parent = if i == 0 {
            1
        } else {
            dirs.iter()
                .position(|&p| p == node.parent)
                .ok_or_else(|| unsupported("missing parent directory"))?
                + 1
        };
        let parent = u16::try_from(parent).map_err(|_| unsupported("too many ISO directories"))?;
        bytes.extend([node.name.len() as u8, 0]);
        bytes.extend(if big {
            node.sector.to_be_bytes()
        } else {
            node.sector.to_le_bytes()
        });
        bytes.extend(if big {
            parent.to_be_bytes()
        } else {
            parent.to_le_bytes()
        });
        bytes.extend(&node.name);
        if node.name.len() % 2 == 1 {
            bytes.push(0);
        }
    }
    Ok(bytes)
}
fn padded(output: &mut impl Write, bytes: &[u8]) -> Result<()> {
    output.write_all(bytes)?;
    let padding = (BLOCK - bytes.len() % BLOCK) % BLOCK;
    output.write_all(&[0; BLOCK][..padding])?;
    Ok(())
}

/// Write a deterministic ISO9660 level 2 image using default options.
pub fn write_iso9660(source: &Path, output: &Path) -> Result<()> {
    write_iso9660_with_options(source, output, &IsoOptions::default())
}

/// Write a configurable ISO9660 image with optional Joliet, Rock Ridge and boot layouts.
/// Rock Ridge encodes symbolic links without following them. Special files are
/// rejected. Output is published only after completion, without overwriting.
pub fn write_iso9660_with_options(
    source: &Path,
    output: &Path,
    options: &IsoOptions,
) -> Result<()> {
    write_iso9660_with_options_and_cancel(source, output, options, || Ok(()))
}

/// Write an ISO with cooperative cancellation checkpoints during payload emission.
/// The callback must not mutate source files. Cancellation leaves no published output.
pub fn write_iso9660_with_options_and_cancel(
    source: &Path,
    output: &Path,
    options: &IsoOptions,
    checkpoint: impl FnMut() -> Result<()>,
) -> Result<()> {
    write_iso9660_with_options_and_progress(source, output, options, checkpoint, |_, _| Ok(()))
}

/// Emit a planned ISO with cooperative cancellation and logical-byte progress.
/// Progress counts each sequential layout byte once; boot patches do not count twice.
/// The byte total does not establish durability: completion still requires synchronization.
pub fn write_iso9660_with_options_and_progress(
    source: &Path,
    output: &Path,
    options: &IsoOptions,
    mut checkpoint: impl FnMut() -> Result<()>,
    mut progress: impl FnMut(u64, u64) -> Result<()>,
) -> Result<()> {
    checkpoint()?;
    options.validate()?;
    if !fs::symlink_metadata(source)?.is_dir() {
        return Err(unsupported("source must be a directory"));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(source)?) {
        return Err(unsupported("output must be outside the source directory"));
    }
    let mut nodes = vec![Node {
        path: source.into(),
        content: None,
        name: vec![0],
        joliet_name: vec![0],
        parent: 0,
        directory: true,
        size: 0,
        sector: 0,
        children: Vec::new(),
        original: String::new(),
        unix: source_metadata(&fs::metadata(source)?, true, false, options),
        link: None,
        identity: None,
        rr_offset: 0,
        rr_length: 0,
    }];
    scan(&mut nodes, 0, 1, 0, options)?;
    emit_nodes(
        source,
        output,
        options,
        nodes,
        &mut checkpoint,
        &mut progress,
        false,
    )
}
fn emit_nodes(
    source: &Path,
    output: &Path,
    options: &IsoOptions,
    mut nodes: Vec<Node>,
    checkpoint: &mut impl FnMut() -> Result<()>,
    progress: &mut impl FnMut(u64, u64) -> Result<()>,
    staged: bool,
) -> Result<()> {
    checkpoint()?;
    let mut identities = HashMap::new();
    let mut serial = 0u32;
    for node in &mut nodes {
        let existing = node
            .identity
            .and_then(|identity| identities.get(&identity).copied());
        let id = if let Some(id) = existing {
            id
        } else {
            serial = serial
                .checked_add(1)
                .ok_or_else(|| unsupported("too many Unix inode identities"))?;
            if let Some(identity) = node.identity {
                identities.insert(identity, serial);
            }
            serial
        };
        node.unix.serial = Some(id);
    }
    let mut links = HashMap::new();
    for node in &nodes {
        if !node.directory {
            *links.entry(node.unix.serial).or_insert(0u32) += 1;
        }
    }
    for index in 0..nodes.len() {
        nodes[index].unix.links = if nodes[index].directory {
            2 + nodes[index]
                .children
                .iter()
                .filter(|&&i| nodes[i].directory)
                .count() as u32
        } else {
            *links
                .get(&nodes[index].unix.serial)
                .ok_or_else(|| unsupported("missing Unix link identity"))?
        };
    }
    let mut rr_pool = Vec::new();
    let mut rr_relocations = Vec::new();
    if options.rock_ridge {
        for node in nodes.iter_mut().skip(1) {
            if node.original.contains('\\')
                || node
                    .link
                    .as_ref()
                    .is_some_and(|link| link.is_empty() || link.len() > 4096 || link.contains('\0'))
            {
                return Err(unsupported("invalid Rock Ridge name or symbolic link"));
            }
            let bytes = crate::rock_ridge::writer::attributes(
                &node.unix,
                options.timestamp.directory_bytes(),
                Some(&node.original),
                node.link.as_deref(),
            );
            (node.rr_offset, node.rr_length) = store_attributes(
                &mut rr_pool,
                &bytes,
                &mut rr_relocations,
                options.max_metadata_bytes,
            )?;
        }
    }
    let mut dirs = vec![0];
    let mut cursor = 0;
    while cursor < dirs.len() {
        let index = dirs[cursor];
        dirs.extend(
            nodes[index]
                .children
                .iter()
                .copied()
                .filter(|&i| nodes[i].directory),
        );
        cursor += 1;
    }
    if dirs.len() > usize::from(u16::MAX) {
        return Err(unsupported("too many ISO directories"));
    }
    let mut joliet_nodes = if options.joliet {
        nodes.clone()
    } else {
        Vec::new()
    };
    for node in &mut joliet_nodes {
        node.name.clone_from(&node.joliet_name);
        node.children.sort_by_key(|&i| nodes[i].joliet_name.clone());
    }
    let mut joliet_dirs = vec![0];
    let mut cursor = 0;
    while options.joliet && cursor < joliet_dirs.len() {
        let index = joliet_dirs[cursor];
        joliet_dirs.extend(
            joliet_nodes[index]
                .children
                .iter()
                .copied()
                .filter(|&i| joliet_nodes[i].directory),
        );
        cursor += 1;
    }
    let table_size = path_table(&nodes, &dirs, false)?.len() as u32;
    let table_blocks = table_size.div_ceil(BLOCK as u32);
    let joliet_size = if options.joliet {
        path_table(&joliet_nodes, &joliet_dirs, false)?.len() as u32
    } else {
        0
    };
    let joliet_blocks = joliet_size.div_ceil(BLOCK as u32);
    let mut boot_entries = Vec::new();
    if let Some(image) = &options.boot.bios {
        let mut entry = BootEntry::bios(image.path.clone());
        entry.image = image.clone();
        boot_entries.push(entry);
    }
    if let Some(image) = &options.boot.efi {
        let mut entry = BootEntry::efi(image.path.clone());
        entry.image = image.clone();
        boot_entries.push(entry);
    }
    boot_entries.extend(options.advanced_boot.entries.iter().cloned());
    if boot_entries.len() > 31 {
        return Err(unsupported("at most 31 El Torito entries"));
    }
    let boot = !boot_entries.is_empty();
    let table_sector = 18 + u32::from(options.joliet) + u32::from(boot);
    let catalog_sector = table_sector + 2 * table_blocks + 2 * joliet_blocks;
    let mut sector = catalog_sector + u32::from(boot);
    for (joliet, tree, tree_dirs) in [
        (false, &mut nodes, &dirs),
        (true, &mut joliet_nodes, &joliet_dirs),
    ] {
        for &i in tree_dirs {
            if tree.is_empty() {
                break;
            }
            tree[i].size = u64::try_from(directory(tree, i, options, joliet, 0)?.len())
                .map_err(|_| unsupported("directory too large"))?;
            tree[i].sector = sector;
            sector = sector
                .checked_add(
                    u32::try_from(tree[i].size.div_ceil(BLOCK as u64))
                        .map_err(|_| unsupported("directory too large"))?,
                )
                .ok_or_else(|| unsupported("volume too large"))?;
        }
    }
    let rr_sector = sector;
    for offset in rr_relocations {
        let relative = u32::from_le_bytes(
            rr_pool[offset..offset + 4]
                .try_into()
                .map_err(|_| unsupported("Rock Ridge relocation"))?,
        );
        number32(
            &mut rr_pool[offset..offset + 8],
            rr_sector
                .checked_add(relative)
                .ok_or_else(|| unsupported("Rock Ridge sector overflow"))?,
        );
    }
    sector = sector
        .checked_add(
            u32::try_from(rr_pool.len().div_ceil(BLOCK))
                .map_err(|_| unsupported("Rock Ridge metadata too large"))?,
        )
        .ok_or_else(|| unsupported("volume too large"))?;
    let metadata_bytes = nodes
        .iter()
        .filter(|node| node.directory)
        .map(|node| node.size)
        .sum::<u64>()
        + joliet_nodes
            .iter()
            .filter(|node| node.directory)
            .map(|node| node.size)
            .sum::<u64>()
        + u64::from(table_size) * 2
        + u64::from(joliet_size) * 2
        + rr_pool.len() as u64;
    if metadata_bytes > options.max_metadata_bytes as u64 {
        return Err(Error::ResourceLimit("ISO writer metadata"));
    }
    let mut file_order: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter_map(|(i, node)| (!node.directory).then_some(i))
        .collect();
    file_order.sort_by_key(|&i| {
        let path = &nodes[i].path;
        !boot_entries
            .iter()
            .any(|entry| source.join(&entry.image.path) == *path)
    });
    let mut allocated = HashMap::new();
    for &i in &file_order {
        let identity = nodes[i].unix.serial;
        if let Some(&previous) = allocated.get(&identity) {
            nodes[i].sector = previous;
        } else {
            nodes[i].sector = sector;
            allocated.insert(identity, sector);
            sector = sector
                .checked_add(
                    u32::try_from(nodes[i].size.div_ceil(BLOCK as u64).max(1))
                        .map_err(|_| unsupported("volume too large"))?,
                )
                .ok_or_else(|| unsupported("volume too large"))?;
        }
        if options.joliet {
            joliet_nodes[i].sector = nodes[i].sector;
        }
    }
    let mut resolved = Vec::new();
    for entry in boot_entries {
        let node = resolve_file(&entry.image.path, source, &nodes)?;
        let size =
            u32::try_from(node.size).map_err(|_| unsupported("boot images must fit 32 bits"))?;
        entry.validate(size)?;
        if entry.emulation == crate::BootEmulation::HardDisk {
            let mut mbr = [0; 512];
            if let Some(content) = &node.content {
                content.read_exact_at(0, &mut mbr)?;
            } else {
                File::open(&node.path)?.read_exact(&mut mbr)?;
            }
            let partitions = mbr[446..510]
                .chunks_exact(16)
                .filter(|part| part[4] != 0)
                .collect::<Vec<_>>();
            if mbr[510..512] != [0x55, 0xaa]
                || partitions.len() != 1
                || partitions[0][4] != entry.system_type
            {
                return Err(unsupported(
                    "hard-disk boot image requires one matching MBR partition",
                ));
            }
        }
        resolved.push((entry, node.sector, size));
    }
    let catalog = if boot {
        Some(advanced_catalog(
            &resolved,
            &options.advanced_boot.catalog_id,
        )?)
    } else {
        None
    };
    let (system_area, hybrid_tail) = if let Some(hybrid) = &options.hybrid {
        let efi = hybrid
            .efi_partition
            .as_ref()
            .map(|path| resolve_file(path, source, &nodes).map(|node| (node.sector, node.size)))
            .transpose()?;
        let bios = resolved
            .iter()
            .find(|(entry, _, _)| entry.platform == 0 && entry.bootable)
            .map(|(_, sector, _)| *sector);
        crate::hybrid::build(hybrid, sector, efi, bios)?
    } else {
        (vec![0; 16 * BLOCK], Vec::new())
    };
    let primary = descriptor(
        &nodes,
        table_size,
        table_sector,
        table_blocks,
        sector,
        false,
        options,
    );
    if u64::from(sector) * BLOCK as u64 + hybrid_tail.len() as u64 > options.max_image_bytes {
        return Err(Error::ResourceLimit("ISO writer image bytes"));
    }
    let total_bytes = u64::from(sector) * BLOCK as u64 + hybrid_tail.len() as u64;
    progress(0, total_bytes)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&system_area)?;
    temporary.write_all(&primary)?;
    if boot {
        temporary.write_all(&boot_descriptor(catalog_sector))?;
    }
    if options.joliet {
        temporary.write_all(&descriptor(
            &joliet_nodes,
            joliet_size,
            table_sector + 2 * table_blocks,
            joliet_blocks,
            sector,
            true,
            options,
        ))?;
    }
    let mut terminator = [0; BLOCK];
    terminator[0] = 255;
    terminator[1..7].copy_from_slice(b"CD001\x01");
    temporary.write_all(&terminator)?;
    for (tree, tree_dirs) in [(&nodes, &dirs), (&joliet_nodes, &joliet_dirs)] {
        if tree.is_empty() {
            continue;
        }
        padded(&mut temporary, &path_table(tree, tree_dirs, false)?)?;
        padded(&mut temporary, &path_table(tree, tree_dirs, true)?)?;
    }
    if let Some(catalog) = catalog {
        temporary.write_all(&catalog)?;
    }
    for (joliet, tree, tree_dirs) in [(false, &nodes, &dirs), (true, &joliet_nodes, &joliet_dirs)] {
        if tree.is_empty() {
            continue;
        }
        for &i in tree_dirs {
            temporary.write_all(&directory(tree, i, options, joliet, rr_sector)?)?;
        }
    }
    padded(&mut temporary, &rr_pool)?;
    let mut emitted = temporary.stream_position()?;
    progress(emitted, total_bytes)?;
    let mut written = HashSet::new();
    // One 1 MiB heap buffer for all payloads: each deferred read re-opens and
    // re-stats the host file, so 64 KiB chunks pay that cost 16x as often.
    let mut buffer = vec![0u8; 1 << 20];
    for &i in &file_order {
        checkpoint()?;
        let node = &nodes[i];
        if !written.insert(node.sector) {
            continue;
        }
        if node.link.is_some() {
            temporary.write_all(&[0; BLOCK])?;
            emitted = emitted
                .checked_add(BLOCK as u64)
                .ok_or(Error::ResourceLimit("emission bytes"))?;
            progress(emitted, total_bytes)?;
            continue;
        }
        let mut file = if node.content.is_none() {
            Some(File::open(&node.path)?)
        } else {
            None
        };
        let mut copied = 0u64;
        loop {
            checkpoint()?;
            let count = if let Some(content) = &node.content {
                let count = usize::try_from((node.size - copied).min(buffer.len() as u64))
                    .map_err(|_| Error::ResourceLimit("payload buffer"))?;
                content.read_exact_at(copied, &mut buffer[..count])?;
                count
            } else {
                file.as_mut().expect("host payload").read(&mut buffer)?
            };
            if count == 0 {
                break;
            }
            copied = copied
                .checked_add(count as u64)
                .ok_or(Error::ResourceLimit("payload bytes"))?;
            if copied > node.size {
                break;
            }
            temporary.write_all(&buffer[..count])?;
            emitted = emitted
                .checked_add(count as u64)
                .ok_or(Error::ResourceLimit("emission bytes"))?;
            progress(emitted, total_bytes)?;
        }
        if copied != node.size {
            return Err(unsupported("source file size changed during creation"));
        }
        let padding = if node.size == 0 {
            BLOCK
        } else {
            let remainder = usize::try_from(node.size % BLOCK as u64)
                .map_err(|_| unsupported("file size padding exceeds address space"))?;
            (BLOCK - remainder % BLOCK) % BLOCK
        };
        temporary.write_all(&[0; BLOCK][..padding])?;
        emitted = emitted
            .checked_add(padding as u64)
            .ok_or(Error::ResourceLimit("emission bytes"))?;
        progress(emitted, total_bytes)?;
    }
    let mut patches = std::collections::BTreeMap::<u32, (BootEntry, u32)>::new();
    for (entry, block, size) in &resolved {
        let (combined, _) = patches
            .entry(*block)
            .or_insert_with(|| (entry.clone(), *size));
        combined.boot_info_table |= entry.boot_info_table;
        combined.grub2_boot_info |= entry.grub2_boot_info;
    }
    for (block, (entry, size)) in patches {
        patch_boot(&mut temporary, &entry, block, size)?;
    }
    temporary.seek(SeekFrom::End(0))?;
    temporary.write_all(&hybrid_tail)?;
    emitted = emitted
        .checked_add(hybrid_tail.len() as u64)
        .ok_or(Error::ResourceLimit("emission bytes"))?;
    if emitted != total_bytes {
        return Err(unsupported("layout emission byte count mismatch"));
    }
    progress(emitted, total_bytes)?;
    checkpoint()?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    checkpoint()?;
    for node in &nodes {
        checkpoint()?;
        if let Some(content) = &node.content {
            content.validate()?;
        }
    }
    if staged {
        temporary
            .persist(output)
            .map_err(|error| Error::Io(error.error))?;
    } else {
        temporary
            .persist_noclobber(output)
            .map_err(|error| Error::Io(error.error))?;
    }
    Ok(())
}

fn joliet_identifier(name: &str, maximum: usize) -> Result<Vec<u8>> {
    if name.is_empty()
        || name.chars().count() > maximum
        || name.chars().any(|c| {
            c as u32 > 0xffff || c.is_control() || matches!(c, '*' | '/' | '\\' | ':' | ';' | '?')
        })
    {
        return Err(unsupported(
            "Joliet names require 1–64 UCS-2 characters without reserved characters",
        ));
    }
    Ok(name.encode_utf16().flat_map(u16::to_be_bytes).collect())
}
fn mangle_identifier(name: &str, directory: bool) -> Vec<u8> {
    if let Ok(identifier) = identifier(name, directory) {
        return identifier;
    }
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else if c == '.' {
                '.'
            } else {
                '_'
            }
        })
        .collect();
    if directory {
        return clean
            .replace('.', "_")
            .chars()
            .take(31)
            .collect::<String>()
            .into_bytes();
    }
    let (base, extension) = clean.rsplit_once('.').unwrap_or((&clean, ""));
    let base: String = base.replace('.', "_").chars().take(22).collect();
    let extension: String = extension.chars().take(8).collect();
    format!(
        "{}.{extension};1",
        if base.is_empty() { "_" } else { &base }
    )
    .into_bytes()
}
fn resolve_file<'a>(path: &Path, source: &Path, nodes: &'a [Node]) -> Result<&'a Node> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(unsupported(
            "boot and partition images require a normal relative path",
        ));
    }
    let path = source.join(path);
    let node = nodes
        .iter()
        .find(|node| node.path == path && !node.directory && node.link.is_none())
        .ok_or_else(|| unsupported("boot image must be a regular source file"))?;
    Ok(node)
}
fn descriptor(
    nodes: &[Node],
    table_size: u32,
    table_sector: u32,
    table_blocks: u32,
    sectors: u32,
    joliet: bool,
    options: &IsoOptions,
) -> [u8; BLOCK] {
    let mut bytes = [0; BLOCK];
    bytes[0] = if joliet { 2 } else { 1 };
    bytes[1..7].copy_from_slice(b"CD001\x01");
    bytes[8..72].fill(b' ');
    if joliet {
        for pair in bytes[8..72].chunks_exact_mut(2) {
            pair.copy_from_slice(&[0, b' ']);
        }
        for (pair, ch) in bytes[40..72]
            .chunks_exact_mut(2)
            .zip(options.volume_label.encode_utf16())
        {
            pair.copy_from_slice(&ch.to_be_bytes());
        }
        bytes[88..91].copy_from_slice(match options.joliet_level {
            1 => b"%/@",
            2 => b"%/C",
            _ => b"%/E",
        });
    } else {
        bytes[40..40 + options.volume_label.len()].copy_from_slice(options.volume_label.as_bytes());
    }
    number32(&mut bytes[80..88], sectors);
    number16(&mut bytes[120..124], 1);
    number16(&mut bytes[124..128], 1);
    number16(&mut bytes[128..132], BLOCK as u16);
    number32(&mut bytes[132..140], table_size);
    bytes[140..144].copy_from_slice(&table_sector.to_le_bytes());
    bytes[148..152].copy_from_slice(&(table_sector + table_blocks).to_be_bytes());
    bytes[156..190].copy_from_slice(&record(&nodes[0], &[0], options));
    bytes[190..813].fill(b' ');
    if joliet {
        for pair in bytes[190..812].chunks_exact_mut(2) {
            pair.copy_from_slice(&[0, b' ']);
        }
    }
    for (start, length, value) in [
        (8, 32, &options.volume_metadata.system_id),
        (190, 128, &options.volume_metadata.volume_set_id),
        (318, 128, &options.volume_metadata.publisher),
        (446, 128, &options.volume_metadata.preparer),
        (574, 128, &options.volume_metadata.application),
    ] {
        if joliet {
            for (pair, unit) in bytes[start..start + length]
                .chunks_exact_mut(2)
                .zip(value.encode_utf16())
            {
                pair.copy_from_slice(&unit.to_be_bytes());
            }
        } else {
            bytes[start..start + value.len()].copy_from_slice(value.as_bytes());
        }
    }
    bytes[847..863].fill(b'0');
    for offset in [813, 830, 864] {
        bytes[offset..offset + 17].copy_from_slice(&options.timestamp.volume_bytes());
    }
    bytes[881] = 1;
    bytes
}

fn patch_boot(
    file: &mut (impl Read + Write + Seek),
    entry: &BootEntry,
    block: u32,
    size: u32,
) -> Result<()> {
    let start = u64::from(block) * BLOCK as u64;
    if entry.grub2_boot_info {
        file.seek(SeekFrom::Start(start + 2548))?;
        file.write_all(&(u64::from(block) * 4 + 5).to_le_bytes())?;
    }
    if entry.boot_info_table {
        file.seek(SeekFrom::Start(start + 64))?;
        let mut left = u64::from(size) - 64;
        let mut checksum = 0u32;
        let mut buffer = [0; 64 * 1024];
        while left != 0 {
            let amount = left.min(buffer.len() as u64) as usize;
            file.read_exact(&mut buffer[..amount])?;
            for chunk in buffer[..amount].chunks(4) {
                let mut word = [0; 4];
                word[..chunk.len()].copy_from_slice(chunk);
                checksum = checksum.wrapping_add(u32::from_le_bytes(word));
            }
            left -= amount as u64;
        }
        let mut table = [0; 56];
        table[..4].copy_from_slice(&16u32.to_le_bytes());
        table[4..8].copy_from_slice(&block.to_le_bytes());
        table[8..12].copy_from_slice(&size.to_le_bytes());
        table[12..16].copy_from_slice(&checksum.to_le_bytes());
        file.seek(SeekFrom::Start(start + 8))?;
        file.write_all(&table)?;
    }
    Ok(())
}

/// Stage a content-only ISO from a bounded deferred inventory. Sparse extents
/// become logical zero bytes; named streams are rejected rather than discarded.
#[allow(clippy::arc_with_non_send_sync)] // Shared ownership supports portable single-threaded reader sources.
pub fn stage_iso9660_from_tree_source(
    source: &impl crate::tree_source::FileTreeSource,
    directory: &Path,
    options: &IsoOptions,
    mut checkpoint: impl FnMut() -> Result<()>,
) -> Result<tempfile::TempPath> {
    checkpoint()?;
    options.validate()?;
    let inventory = source.inventory(options.max_entries, options.max_metadata_bytes)?;
    inventory.validate_budget(options.max_entries, options.max_metadata_bytes)?;
    stage_inventory(inventory, directory, options, checkpoint)
}
#[allow(clippy::arc_with_non_send_sync)] // Retained portable reader ownership.
fn stage_inventory(
    inventory: crate::tree_source::TreeInventory,
    directory: &Path,
    options: &IsoOptions,
    mut checkpoint: impl FnMut() -> Result<()>,
) -> Result<tempfile::TempPath> {
    use crate::tree_source::{DeferredContent, ExtentContent, TreeEntryKind};
    use std::sync::Arc;
    if !inventory.root_streams.is_empty() || !inventory.system_streams.is_empty() {
        return Err(unsupported("ISO cannot preserve root or system streams"));
    }
    let unix = |directory: bool, link: bool| crate::rock_ridge::UnixMetadata {
        mode: if directory {
            0o040000 | options.unix_metadata.directory_mode
        } else if link {
            0o120777
        } else {
            0o100000 | options.unix_metadata.file_mode
        },
        links: 1,
        uid: options.unix_metadata.uid,
        gid: options.unix_metadata.gid,
        serial: None,
        device: None,
        timestamps: Vec::new(),
    };
    let mut nodes = vec![Node {
        path: PathBuf::new(),
        content: None,
        name: vec![0],
        joliet_name: vec![0],
        parent: 0,
        directory: true,
        size: 0,
        sector: 0,
        children: Vec::new(),
        original: String::new(),
        unix: unix(true, false),
        link: None,
        identity: None,
        rr_offset: 0,
        rr_length: 0,
    }];
    let mut entries = inventory.entries;
    entries.sort_by(|a, b| {
        a.path
            .split('/')
            .count()
            .cmp(&b.path.split('/').count())
            .then(a.path.cmp(&b.path))
    });
    let mut paths = HashMap::from([(String::new(), 0usize)]);
    for entry in entries {
        checkpoint()?;
        if !entry.streams.is_empty() {
            return Err(unsupported("ISO cannot preserve named streams"));
        }
        if entry.path.starts_with('/')
            || entry
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(unsupported("invalid source path"));
        }
        let (parent_path, original) = entry.path.rsplit_once('/').unwrap_or(("", &entry.path));
        let parent = *paths
            .get(parent_path)
            .ok_or_else(|| unsupported("missing source parent directory"))?;
        if !nodes[parent].directory {
            return Err(unsupported("source parent is not a directory"));
        }
        let (directory, link, content, identity) = match entry.kind {
            TreeEntryKind::Directory => (true, None, None, None),
            TreeEntryKind::Symlink(target) => {
                if !options.rock_ridge {
                    return Err(unsupported("symlinks require Rock Ridge"));
                }
                (false, Some(target), None, None)
            }
            TreeEntryKind::File(extents) => (
                false,
                None,
                Some(DeferredContent::new(Arc::new(ExtentContent::new(
                    entry.object,
                    extents,
                )?))),
                Some(((entry.object >> 64) as u64, entry.object as u64)),
            ),
            TreeEntryKind::HardLink(target) => {
                let index = *paths
                    .get(&target)
                    .ok_or_else(|| unsupported("hard-link target must precede alias"))?;
                if nodes[index].directory || nodes[index].link.is_some() {
                    return Err(unsupported("invalid hard-link target"));
                }
                (
                    false,
                    None,
                    nodes[index].content.clone(),
                    nodes[index].identity,
                )
            }
        };
        let size = content.as_ref().map_or(0, DeferredContent::len);
        if options.level != IsoLevel::Level3 && size > u64::from(u32::MAX) {
            return Err(unsupported("files larger than 4 GiB require ISO level 3"));
        }
        let name = if options.filename_policy == FilenamePolicy::Mangle {
            mangle_for_level(original, directory, options.level)
        } else {
            identifier_for_level(original, directory, options.level)?
        };
        if entry.path.split('/').count() >= options.max_directory_depth
            || entry.path.len() > options.max_path_bytes
        {
            return Err(unsupported("ISO path or depth limit exceeded"));
        }
        let joliet_name = if options.joliet {
            joliet_identifier(original, options.joliet_max_name)?
        } else {
            Vec::new()
        };
        let index = nodes.len();
        nodes.push(Node {
            path: PathBuf::from(&entry.path),
            content,
            name,
            joliet_name,
            parent,
            directory,
            size,
            sector: 0,
            children: Vec::new(),
            original: original.into(),
            unix: unix(directory, link.is_some()),
            link,
            identity,
            rr_offset: 0,
            rr_length: 0,
        });
        nodes[parent].children.push(index);
        if paths.insert(entry.path, index).is_some() {
            return Err(unsupported("duplicate source path"));
        }
    }
    for parent in 0..nodes.len() {
        let children = nodes[parent].children.clone();
        let mut primary = HashSet::new();
        let mut joliet = HashSet::new();
        for (ordinal, child) in children.into_iter().enumerate() {
            assign_primary_name(&mut nodes[child], ordinal, &mut primary, options)?;
            if options.joliet && !joliet.insert(nodes[child].joliet_name.clone()) {
                return Err(unsupported("Joliet destination name collision"));
            }
        }
    }
    let (file, path) = tempfile::NamedTempFile::new_in(directory)?.into_parts();
    drop(file);
    emit_nodes(
        Path::new(""),
        &path,
        options,
        nodes,
        &mut checkpoint,
        &mut |_, _| Ok(()),
        true,
    )?;
    Ok(path)
}

/// Stage after explicit metadata preflight, returning the transformation report.
pub fn stage_iso9660_from_tree_source_with_policy(
    source: &impl crate::tree_source::FileTreeSource,
    directory: &Path,
    options: &IsoOptions,
    policy: crate::preservation::Policy,
    mut checkpoint: impl FnMut() -> Result<()>,
) -> Result<(tempfile::TempPath, crate::preservation::Report)> {
    checkpoint()?;
    options.validate()?;
    let inventory = source.inventory(options.max_entries, options.max_metadata_bytes)?;
    inventory.validate_budget(options.max_entries, options.max_metadata_bytes)?;
    let profile = if options.rock_ridge {
        crate::preservation::Profile::IsoRockRidge
    } else if options.joliet {
        crate::preservation::Profile::IsoJoliet
    } else {
        crate::preservation::Profile::IsoPrimary
    };
    let metadata = inventory
        .entries
        .iter()
        .flat_map(|entry| {
            std::iter::once(&entry.metadata)
                .chain(entry.streams.iter().map(|stream| &stream.metadata))
        })
        .chain(
            inventory
                .root_streams
                .iter()
                .chain(&inventory.system_streams)
                .map(|stream| &stream.metadata),
        );
    let report =
        crate::preservation::preflight_iter(profile, policy, &inventory.root_metadata, metadata);
    if !report.allowed() {
        return Err(unsupported(
            "preservation preflight rejected requested policy",
        ));
    }
    Ok((
        stage_inventory(inventory, directory, options, checkpoint)?,
        report,
    ))
}
