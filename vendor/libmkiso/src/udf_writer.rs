//! Configurable, deterministic UDF image authoring.
//!
//! Metadata is planned before publication; file payloads are copied in bounded chunks.
use crate::{BootOptions, IsoTimestamp};
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path},
    sync::Arc,
};
const BLOCK: u64 = 2048;
const PART: u32 = 320;
const MAX_EXTENT: u64 = 0x3ffff800;

/// UDF revision recorded in domain and implementation identifiers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UdfRevision {
    #[default]
    V102,
    V150,
    V200,
    V201,
    V250,
    V260,
}
impl UdfRevision {
    /// The binary-coded UDF revision.
    pub fn number(self) -> u16 {
        match self {
            Self::V102 => 0x102,
            Self::V150 => 0x150,
            Self::V200 => 0x200,
            Self::V201 => 0x201,
            Self::V250 => 0x250,
            Self::V260 => 0x260,
        }
    }
}
/// Partition address translation used by the authored image.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UdfPartition {
    /// Read-only physical partition.
    #[default]
    Physical,
    /// Separate physical partitions for namespace metadata and file payloads.
    PhysicalSplit,
    /// Write-once virtual partition with a final VAT.
    Virtual,
    /// Rewritable packet partition with redundant sparing tables.
    Sparable { packet_blocks: u16 },
    /// Read-only UDF 2.50/2.60 metadata partition, optionally duplicated.
    Metadata { mirror: bool },
    /// Metadata partition backed by a sparable physical map.
    MetadataSparable { mirror: bool, packet_blocks: u16 },
}
/// File allocation descriptor encoding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AllocationMode {
    #[default]
    Short,
    Long,
    Extended,
    Embedded,
}
/// ICB encoding for directory references.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IcbStrategy {
    /// Strategy 4 direct entries.
    #[default]
    Direct,
    /// An indirect entry preceding the strategy 4 file entry.
    Indirect,
    /// Strategy 4096 two-block ICBs terminated by a terminal entry.
    Strategy4096,
}
/// Configuration for native UDF image creation.
#[derive(Debug, Clone)]
pub struct UdfOptions {
    pub revision: UdfRevision,
    pub partition: UdfPartition,
    pub allocation: AllocationMode,
    pub label: String,
    pub timestamp: IsoTimestamp,
    pub boot: BootOptions,
    pub icb_strategy: IcbStrategy,
    /// Number of linked file-set descriptors (the last prevails).
    pub file_set_descriptors: u16,
    /// Maximum number of authored objects, including stream directories.
    pub max_entries: usize,
    /// Maximum cumulative in-memory metadata and allocation descriptor bytes.
    pub max_metadata_bytes: u64,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    pub max_image_bytes: u64,
    pub max_nesting_depth: usize,
    /// Maximum blocks per allocation extent; small values exercise AED chains.
    pub extent_blocks: u32,
    /// Metadata extent size in blocks; zero is contiguous, otherwise a multiple of 32.
    pub metadata_extent_blocks: u32,
    /// Logical packet starts to relocate into the sparable profile's replacement area.
    pub sparing_packets: Vec<u32>,
}
impl Default for UdfOptions {
    fn default() -> Self {
        Self {
            revision: UdfRevision::V102,
            partition: UdfPartition::Physical,
            allocation: AllocationMode::Short,
            label: "UDFIMAGE".into(),
            timestamp: IsoTimestamp::default(),
            boot: BootOptions::default(),
            icb_strategy: IcbStrategy::Direct,
            file_set_descriptors: 1,
            max_entries: 100_000,
            max_metadata_bytes: 64 << 20,
            max_entry_bytes: 1 << 40,
            max_total_bytes: 4 << 40,
            max_image_bytes: 8 << 40,
            max_nesting_depth: 64,
            extent_blocks: (MAX_EXTENT / BLOCK) as u32,
            metadata_extent_blocks: 0,
            sparing_packets: Vec::new(),
        }
    }
}
impl UdfOptions {
    fn validate(&self) -> Result<()> {
        self.timestamp.validate()?;
        ensure!(
            !self.label.is_empty() && compressed(&self.label)?.len() <= 31,
            "UDF volume label exceeds 31 encoded bytes"
        );
        ensure!(
            self.extent_blocks > 0 && u64::from(self.extent_blocks) * BLOCK <= MAX_EXTENT,
            "invalid allocation extent block limit"
        );
        ensure!(
            self.max_entries > 0 && (1..=64).contains(&self.file_set_descriptors),
            "invalid writer metadata limits"
        );
        match self.partition {
            UdfPartition::Virtual => {
                ensure!(
                    self.revision.number() >= 0x150,
                    "VAT requires UDF 1.50 or newer"
                );
                ensure!(
                    self.revision.number() <= 0x201,
                    "UDF 2.50/2.60 uses metadata partitions, not virtual partitions"
                );
                ensure!(
                    matches!(
                        self.allocation,
                        AllocationMode::Long | AllocationMode::Embedded
                    ),
                    "virtual partitions require long file allocations"
                );
                ensure!(
                    self.icb_strategy != IcbStrategy::Strategy4096,
                    "strategy 4096 requires nonsequential write-once media"
                );
            }
            UdfPartition::Metadata { .. } | UdfPartition::MetadataSparable { .. } => {
                ensure!(
                    self.revision.number() >= 0x250,
                    "metadata partitions require UDF 2.50 or 2.60"
                );
                ensure!(
                    matches!(
                        self.allocation,
                        AllocationMode::Long | AllocationMode::Embedded
                    ),
                    "metadata partitions require physical long file allocations"
                );
                ensure!(
                    self.icb_strategy != IcbStrategy::Strategy4096,
                    "strategy 4096 is not a read-only metadata partition profile"
                );
            }
            UdfPartition::Sparable { packet_blocks } => {
                ensure!(
                    self.revision.number() >= 0x150 && self.revision.number() <= 0x201,
                    "sparable writer profile requires UDF 1.50 through 2.01"
                );
                ensure!(
                    packet_blocks != 0 && packet_blocks.is_power_of_two(),
                    "invalid sparing packet size"
                );
                ensure!(
                    self.icb_strategy != IcbStrategy::Strategy4096,
                    "strategy 4096 requires write-once partition access"
                );
            }
            UdfPartition::PhysicalSplit => {
                ensure!(
                    matches!(
                        self.allocation,
                        AllocationMode::Long | AllocationMode::Extended | AllocationMode::Embedded
                    ),
                    "split physical partitions require partition-qualified allocations"
                );
            }
            UdfPartition::Physical => {}
        }
        if let UdfPartition::Sparable { packet_blocks }
        | UdfPartition::MetadataSparable { packet_blocks, .. } = self.partition
        {
            ensure!(
                packet_blocks != 0
                    && packet_blocks.is_power_of_two()
                    && PART.is_multiple_of(u32::from(packet_blocks)),
                "invalid sparing packet alignment"
            );
        }
        ensure!(
            self.metadata_extent_blocks == 0
                || (self.metadata_extent_blocks.is_multiple_of(32)
                    && u64::from(self.metadata_extent_blocks) * BLOCK <= MAX_EXTENT),
            "invalid metadata extent block size"
        );
        ensure!(
            self.metadata_extent_blocks == 0
                || matches!(
                    self.partition,
                    UdfPartition::Metadata { .. } | UdfPartition::MetadataSparable { .. }
                ),
            "fragmented metadata requires a metadata partition"
        );
        ensure!(
            self.sparing_packets.is_empty()
                || matches!(
                    self.partition,
                    UdfPartition::Sparable { .. } | UdfPartition::MetadataSparable { .. }
                ),
            "sparing replacements require a sparable partition"
        );
        ensure!(
            self.sparing_packets.len() <= (65536 - 56) / 8,
            "sparing table exceeds its reserved area"
        );
        ensure!(
            self.file_set_descriptors == 1
                || matches!(
                    self.partition,
                    UdfPartition::Physical | UdfPartition::PhysicalSplit
                ),
            "multiple file sets require the nonsequential write-once physical profile"
        );
        Ok(())
    }
}
/// A regular file's explicit recorded or unallocated extent.
#[derive(Debug, Clone)]
pub enum UdfFileExtent {
    Data(Vec<u8>),
    Hole(u64),
    /// Allocated, unrecorded blocks that read as zeroes.
    AllocatedHole(u64),
}
#[derive(Debug, Clone)]
enum Piece {
    Deferred(crate::tree_source::DeferredContent),
    Hole(u64),
    AllocatedHole(u64),
}
#[derive(Debug, Clone)]
enum Payload {
    Pieces(Arc<[Piece]>),
    Link(Arc<str>),
    HardLink(String),
    Directory,
}
#[derive(Debug, Clone)]
struct Input {
    payload: Payload,
    preallocated: u32,
    streams: BTreeMap<String, Payload>,
}
/// An explicit image tree, including streams, links and sparse data.
#[derive(Debug, Clone, Default)]
pub struct UdfImage {
    entries: BTreeMap<String, Input>,
    system_streams: BTreeMap<String, Payload>,
    root_streams: BTreeMap<String, Payload>,
}
impl UdfImage {
    /// Create an empty image tree.
    pub fn new() -> Self {
        Self::default()
    }
    fn insert(&mut self, path: &str, payload: Payload) -> Result<()> {
        let path = normal_path(path)?;
        ensure!(
            !self.entries.contains_key(&path),
            "duplicate UDF path: {path}"
        );
        let mut parent = path.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if let Some(previous) = self.entries.get(prefix) {
                ensure!(
                    matches!(previous.payload, Payload::Directory),
                    "parent is not a directory: {prefix}"
                );
            } else {
                self.entries.insert(
                    prefix.to_owned(),
                    Input {
                        payload: Payload::Directory,
                        preallocated: 0,
                        streams: BTreeMap::new(),
                    },
                );
            }
            parent = prefix;
        }
        self.entries.insert(
            path,
            Input {
                payload,
                preallocated: 0,
                streams: BTreeMap::new(),
            },
        );
        Ok(())
    }
    /// Add a file copied from a host file at write time.
    pub fn add_file(&mut self, path: impl AsRef<str>, source: impl AsRef<Path>) -> Result<()> {
        let source = source.as_ref().to_path_buf();
        let metadata = fs::symlink_metadata(&source)?;
        ensure!(metadata.is_file(), "source is not a regular file");
        self.insert(
            path.as_ref(),
            Payload::Pieces(Arc::from([Piece::Deferred(
                crate::tree_source::host_file_content(source)?,
            )])),
        )
    }
    /// Add a regular file whose bytes are owned by the builder.
    pub fn add_bytes(&mut self, path: impl AsRef<str>, bytes: Vec<u8>) -> Result<()> {
        self.insert(
            path.as_ref(),
            Payload::Pieces(Arc::from([Piece::Deferred(
                crate::tree_source::DeferredContent::new(Arc::new(
                    crate::tree_source::BufferContent::new(0, bytes),
                )),
            )])),
        )
    }
    /// Add an empty directory (parents are created automatically).
    pub fn add_directory(&mut self, path: impl AsRef<str>) -> Result<()> {
        let path = normal_path(path.as_ref())?;
        if self
            .entries
            .get(&path)
            .is_some_and(|p| matches!(p.payload, Payload::Directory))
        {
            return Ok(());
        }
        self.insert(&path, Payload::Directory)
    }
    /// Add recorded segments and sparse holes. Interior segments must be block aligned.
    pub fn add_sparse_file(
        &mut self,
        path: impl AsRef<str>,
        extents: Vec<UdfFileExtent>,
    ) -> Result<()> {
        self.insert(
            path.as_ref(),
            Payload::Pieces(
                extents
                    .into_iter()
                    .map(|e| match e {
                        UdfFileExtent::Data(b) => {
                            Piece::Deferred(crate::tree_source::DeferredContent::new(Arc::new(
                                crate::tree_source::BufferContent::new(0, b),
                            )))
                        }
                        UdfFileExtent::Hole(n) => Piece::Hole(n),
                        UdfFileExtent::AllocatedHole(n) => Piece::AllocatedHole(n),
                    })
                    .collect::<Vec<_>>()
                    .into(),
            ),
        )
    }
    /// Add an encoded ECMA-167 symbolic link without following its target.
    pub fn add_symlink(&mut self, path: impl AsRef<str>, target: impl Into<String>) -> Result<()> {
        let target = target.into();
        link_bytes(&target)?;
        self.insert(path.as_ref(), Payload::Link(Arc::from(target)))
    }
    /// Add a second name for an existing regular file ICB.
    pub fn add_hard_link(&mut self, path: impl AsRef<str>, target: impl AsRef<str>) -> Result<()> {
        self.insert(
            path.as_ref(),
            Payload::HardLink(normal_path(target.as_ref())?),
        )
    }
    /// Attach a named stream to a file or directory.
    pub fn add_named_stream(
        &mut self,
        owner: impl AsRef<str>,
        name: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<()> {
        let name = name.into();
        component(&name)?;
        ensure!(!name.starts_with("*UDF"), "reserved UDF stream name");
        if owner.as_ref().is_empty() {
            ensure!(
                !self.root_streams.contains_key(&name),
                "duplicate root stream name"
            );
            self.root_streams.insert(
                name,
                Payload::Pieces(Arc::from([Piece::Deferred(
                    crate::tree_source::DeferredContent::new(Arc::new(
                        crate::tree_source::BufferContent::new(0, bytes),
                    )),
                )])),
            );
            return Ok(());
        }
        let owner = normal_path(owner.as_ref())?;
        let entry = self
            .entries
            .get_mut(&owner)
            .context("named stream owner does not exist")?;
        ensure!(
            !matches!(entry.payload, Payload::HardLink(_)),
            "attach streams to the hard-link target"
        );
        ensure!(!entry.streams.contains_key(&name), "duplicate stream name");
        entry.streams.insert(
            name,
            Payload::Pieces(Arc::from([Piece::Deferred(
                crate::tree_source::DeferredContent::new(Arc::new(
                    crate::tree_source::BufferContent::new(0, bytes),
                )),
            )])),
        );
        Ok(())
    }
    /// Attach a named stream to the root directory.
    pub fn add_root_stream(&mut self, name: impl Into<String>, bytes: Vec<u8>) -> Result<()> {
        self.add_named_stream("", name, bytes)
    }
    /// Add a stream in the file set's system stream directory.
    pub fn add_system_stream(&mut self, name: impl Into<String>, bytes: Vec<u8>) -> Result<()> {
        let name = name.into();
        component(&name)?;
        ensure!(!name.starts_with("*UDF"), "reserved UDF stream name");
        ensure!(
            !self.system_streams.contains_key(&name),
            "duplicate system stream"
        );
        self.system_streams.insert(
            name,
            Payload::Pieces(Arc::from([Piece::Deferred(
                crate::tree_source::DeferredContent::new(Arc::new(
                    crate::tree_source::BufferContent::new(0, bytes),
                )),
            )])),
        );
        Ok(())
    }
    /// Allocate unrecorded blocks beyond a file's Information Length.
    pub fn set_preallocated_blocks(&mut self, path: impl AsRef<str>, blocks: u32) -> Result<()> {
        let path = normal_path(path.as_ref())?;
        let input = self
            .entries
            .get_mut(&path)
            .context("preallocation owner does not exist")?;
        ensure!(
            !matches!(input.payload, Payload::Directory | Payload::HardLink(_)),
            "preallocation requires a file"
        );
        input.preallocated = blocks;
        Ok(())
    }
    /// Preflight an explicit preservation policy before translating a source.
    /// Faithful capture is gated when any native field cannot be emitted.
    pub fn from_tree_source_with_policy(
        source: &impl crate::tree_source::FileTreeSource,
        maximum_entries: usize,
        maximum_bytes: usize,
        profile: crate::preservation::Profile,
        policy: crate::preservation::Policy,
    ) -> Result<(Self, crate::preservation::Report)> {
        let inventory = source.inventory(maximum_entries, maximum_bytes)?;
        inventory.validate_budget(maximum_entries, maximum_bytes)?;
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
        let report = crate::preservation::preflight_iter(
            profile,
            policy,
            &inventory.root_metadata,
            metadata,
        );
        ensure!(
            report.allowed(),
            "preservation preflight rejected requested policy: {:?}",
            report.issues
        );
        Ok((Self::from_inventory(inventory)?, report))
    }
    /// Translate a bounded, content-only source inventory without reading payloads.
    pub fn from_tree_source(
        source: &impl crate::tree_source::FileTreeSource,
        maximum_entries: usize,
        maximum_bytes: usize,
    ) -> Result<Self> {
        let inventory = source.inventory(maximum_entries, maximum_bytes)?;
        inventory.validate_budget(maximum_entries, maximum_bytes)?;
        Self::from_inventory(inventory)
    }
    fn from_inventory(inventory: crate::tree_source::TreeInventory) -> Result<Self> {
        use crate::tree_source::{TreeEntryKind, TreeExtent};
        let mut image = Self::new();
        let stream_payload = |content| Payload::Pieces(Arc::from([Piece::Deferred(content)]));
        let mut identities: BTreeMap<u128, String> = BTreeMap::new();
        for entry in inventory.entries {
            let payload = match entry.kind {
                TreeEntryKind::Directory => Payload::Directory,
                TreeEntryKind::Symlink(target) => Payload::Link(Arc::from(target)),
                TreeEntryKind::HardLink(target) => Payload::HardLink(normal_path(&target)?),
                TreeEntryKind::File(extents) => {
                    if let Some(target) = identities.get(&entry.object) {
                        ensure!(entry.streams.is_empty(), "hard-link alias carries streams");
                        Payload::HardLink(target.clone())
                    } else {
                        identities.insert(entry.object, normal_path(&entry.path)?);
                        Payload::Pieces(
                            extents
                                .into_iter()
                                .map(|extent| match extent {
                                    TreeExtent::Data(content) => Piece::Deferred(content),
                                    TreeExtent::Hole(length) => Piece::Hole(length),
                                    TreeExtent::AllocatedHole(length) => {
                                        Piece::AllocatedHole(length)
                                    }
                                })
                                .collect::<Vec<_>>()
                                .into(),
                        )
                    }
                }
            };
            image.insert(&entry.path, payload)?;
            for stream in entry.streams {
                component(&stream.name)?;
                ensure!(!stream.name.starts_with("*UDF"), "reserved UDF stream name");
                let owner = image
                    .entries
                    .get_mut(&normal_path(&entry.path)?)
                    .context("stream owner missing")?;
                ensure!(
                    owner
                        .streams
                        .insert(stream.name, stream_payload(stream.content))
                        .is_none(),
                    "duplicate stream"
                );
            }
        }
        for (streams, destination) in [
            (inventory.root_streams, &mut image.root_streams),
            (inventory.system_streams, &mut image.system_streams),
        ] {
            for stream in streams {
                component(&stream.name)?;
                ensure!(!stream.name.starts_with("*UDF"), "reserved UDF stream name");
                ensure!(
                    destination
                        .insert(stream.name, stream_payload(stream.content))
                        .is_none(),
                    "duplicate stream"
                );
            }
        }
        Ok(image)
    }
    /// Emit a closed, unpublished image for caller inspection before publication.
    /// The returned temporary file is deleted on drop unless explicitly retained.
    pub fn stage_with_cancel(
        &self,
        directory: &Path,
        options: &UdfOptions,
        mut checkpoint: impl FnMut() -> Result<()>,
    ) -> Result<(tempfile::TempPath, String)> {
        let (file, path) = tempfile::NamedTempFile::new_in(directory)?.into_parts();
        drop(file);
        write_image(self, &path, options, &mut checkpoint, true).map(|hash| (path, hash))
    }
    /// Write an image and return its SHA-256 digest, refusing to overwrite output.
    pub fn write(&self, output: &Path, options: &UdfOptions) -> Result<String> {
        self.write_with_cancel(output, options, || Ok(()))
    }
    /// Write with cancellation checkpoints during layout, copies, hashing and publication.
    pub fn write_with_cancel(
        &self,
        output: &Path,
        options: &UdfOptions,
        mut checkpoint: impl FnMut() -> Result<()>,
    ) -> Result<String> {
        write_image(self, output, options, &mut checkpoint, false)
    }
}
/// Write a source directory with the default read-only UDF 1.02 profile.
pub fn write_udf(source: &Path, output: &Path) -> Result<()> {
    write_udf_with_options(source, output, &UdfOptions::default()).map(|_| ())
}
/// Write a source directory using the requested revision and partition profile.
pub fn write_udf_with_options(
    source: &Path,
    output: &Path,
    options: &UdfOptions,
) -> Result<String> {
    write_udf_with_cancel(source, output, options, || Ok(()))
}
/// Scan and write a source tree with cooperative cancellation throughout.
pub fn write_udf_with_cancel(
    source: &Path,
    output: &Path,
    options: &UdfOptions,
    mut checkpoint: impl FnMut() -> Result<()>,
) -> Result<String> {
    checkpoint()?;
    options.validate()?;
    ensure!(!output.exists(), "output already exists");
    let root = source.canonicalize()?;
    ensure!(root.is_dir(), "source must be a directory");
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    ensure!(
        !parent.starts_with(&root),
        "output must be outside source tree"
    );
    let mut image = UdfImage::new();
    let mut queue = vec![root.clone()];
    let mut scanned_bytes = 0u64;
    while let Some(dir) = queue.pop() {
        checkpoint()?;
        let mut paths = Vec::new();
        for entry in fs::read_dir(dir)? {
            checkpoint()?;
            ensure!(
                image.entries.len() + paths.len() + 1 < options.max_entries,
                "writer entry limit exceeded"
            );
            let path = entry?.path();
            scanned_bytes = scanned_bytes
                .checked_add(path.as_os_str().len() as u64)
                .context("scan metadata overflow")?;
            ensure!(
                scanned_bytes <= options.max_metadata_bytes,
                "writer scan metadata limit exceeded"
            );
            paths.push(path);
        }
        paths.sort();
        for path in paths {
            checkpoint()?;
            let relative = host_relative_path(path.strip_prefix(&root)?)?;
            ensure!(
                relative.split('/').count() <= options.max_nesting_depth,
                "writer nesting limit exceeded"
            );
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                image.add_directory(&relative)?;
                queue.push(path);
            } else if metadata.is_file() {
                image.add_file(&relative, path)?;
            } else if metadata.is_symlink() {
                let target = fs::read_link(&path)?;
                let target = target.to_str().context("symlink target is not Unicode")?;
                #[cfg(windows)]
                let target = target.replace('\\', "/");
                #[cfg(not(windows))]
                let target = target.to_owned();
                image.add_symlink(&relative, target)?;
            } else {
                bail!("unsupported special source file");
            }
            ensure!(
                image.entries.len() < options.max_entries,
                "writer entry limit exceeded"
            );
        }
    }
    image.write_with_cancel(output, options, checkpoint)
}
fn normal_path(path: &str) -> Result<String> {
    ensure!(
        !path.is_empty() && !path.contains(['\\', '\0']),
        "invalid UDF relative path"
    );
    ensure!(
        !path.starts_with('/')
            && path
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != ".."),
        "UDF path must have only normal components"
    );
    for part in path.split('/') {
        component(part)?;
    }
    Ok(path.to_owned())
}
fn host_relative_path(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for part in path.components() {
        let Component::Normal(part) = part else {
            bail!("path must be relative");
        };
        let text = part.to_str().context("filename is not Unicode")?;
        component(text)?;
        parts.push(text);
    }
    normal_path(&parts.join("/"))
}
fn component(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty() && !s.contains(['/', '\\', '\0']) && s != "." && s != "..",
        "invalid UDF name"
    );
    ensure!(
        compressed(s)?.len() <= 255,
        "UDF identifier exceeds 255 bytes"
    );
    Ok(())
}
fn compressed(s: &str) -> Result<Vec<u8>> {
    ensure!(!s.contains('\0'), "NUL in UDF identifier");
    let units: Vec<_> = s.encode_utf16().collect();
    let mut b = Vec::new();
    if units.iter().all(|u| *u <= 255) {
        b.push(8);
        b.extend(units.into_iter().map(|u| u as u8));
    } else {
        b.push(16);
        for unit in units {
            b.extend(unit.to_be_bytes());
        }
    }
    Ok(b)
}
fn link_bytes(s: &str) -> Result<Vec<u8>> {
    ensure!(
        !s.is_empty() && !s.contains(['\\', '\0']),
        "invalid symbolic link target"
    );
    let mut b = Vec::new();
    if s.starts_with('/') {
        b.extend([1, 0, 0, 0]);
    }
    for part in s.split('/').filter(|p| !p.is_empty()) {
        match part {
            "." => b.extend([4, 0, 0, 0]),
            ".." => b.extend([3, 0, 0, 0]),
            _ => {
                component(part)?;
                let name = compressed(part)?;
                b.extend([5, name.len() as u8, 0, 0]);
                b.extend(name);
            }
        }
    }
    Ok(b)
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
fn crc(bytes: &[u8]) -> u16 {
    let mut c = 0u16;
    for x in bytes {
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
fn tag(b: &mut [u8], id: u16, loc: u32, len: usize, revision: UdfRevision) {
    put16(b, 0, id);
    put16(b, 2, if revision.number() >= 0x200 { 3 } else { 2 });
    put16(b, 6, 1);
    put16(b, 8, crc(&b[16..len]));
    put16(b, 10, (len - 16) as u16);
    put32(b, 12, loc);
    b[4] = b[..16]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |a, (_, v)| a.wrapping_add(*v));
}
fn reg(b: &mut [u8], p: usize, id: &[u8], revision: Option<UdfRevision>) {
    b[p + 1..p + 1 + id.len()].copy_from_slice(id);
    if let Some(revision) = revision {
        put16(b, p + 24, revision.number());
    }
}
fn chars(b: &mut [u8], p: usize) {
    b[p + 1..p + 24].copy_from_slice(b"OSTA Compressed Unicode");
}
fn dstring(b: &mut [u8], p: usize, len: usize, s: &str) -> Result<()> {
    let text = compressed(s)?;
    ensure!(text.len() < len, "UDF dstring exceeds field");
    b[p..p + text.len()].copy_from_slice(&text);
    b[p + len - 1] = text.len() as u8;
    Ok(())
}
fn timestamp(b: &mut [u8], p: usize, t: IsoTimestamp) {
    put16(b, p, 0x1000);
    put16(b, p + 2, t.year);
    b[p + 4..p + 10].copy_from_slice(&[t.month, t.day, t.hour, t.minute, t.second, 0]);
}
fn long_ad(b: &mut [u8], p: usize, length: u32, location: u32, partition: u16) {
    put32(b, p, length);
    put32(b, p + 4, location);
    put16(b, p + 8, partition);
}
fn blocks(size: u64) -> Result<u32> {
    u32::try_from(size.div_ceil(BLOCK)).context("image exceeds 32-bit block addresses")
}
fn take(cursor: &mut u32, length: u32) -> Result<u32> {
    let start = *cursor;
    *cursor = cursor
        .checked_add(length)
        .context("image layout overflow")?;
    Ok(start)
}

#[derive(Debug, Clone)]
struct Node {
    path: String,
    name: Vec<u8>,
    parent: usize,
    children: Vec<usize>,
    payload: Payload,
    stream_dir: Option<usize>,
    system: bool,
    stream_owner: Option<usize>,
    system_stream: bool,
    links: u16,
    preallocated: u32,
    alias: Option<usize>,
    icb: u32,
    entry: u32,
    data: u32,
    size: u64,
    mode: AllocationMode,
    allocations: Vec<Ad>,
    aeds: Vec<u32>,
    embedded: Vec<u8>,
}
#[derive(Debug, Clone, Copy)]
struct Ad {
    kind: u32,
    length: u32,
    information: u32,
    block: u32,
    partition: u16,
}
impl Node {
    fn directory(&self) -> bool {
        matches!(self.payload, Payload::Directory)
    }
    fn file_type(&self) -> u8 {
        if self.directory() {
            if self.system { 13 } else { 4 }
        } else if matches!(self.payload, Payload::Link(_)) {
            12
        } else {
            5
        }
    }
}
fn node(path: String, name: Vec<u8>, parent: usize, payload: Payload) -> Node {
    Node {
        path,
        name,
        parent,
        children: Vec::new(),
        payload,
        stream_dir: None,
        system: false,
        stream_owner: None,
        system_stream: false,
        links: 0,
        preallocated: 0,
        alias: None,
        icb: 0,
        entry: 0,
        data: 0,
        size: 0,
        mode: AllocationMode::Short,
        allocations: Vec::new(),
        aeds: Vec::new(),
        embedded: Vec::new(),
    }
}
fn build_nodes(image: &UdfImage, options: &UdfOptions) -> Result<(Vec<Node>, Option<usize>)> {
    let extra_nodes = image.entries.values().try_fold(0usize, |sum, input| {
        sum.checked_add(input.streams.len() + usize::from(!input.streams.is_empty()))
            .context("stream count overflow")
    })? + image.root_streams.len()
        + usize::from(!image.root_streams.is_empty())
        + image.system_streams.len()
        + usize::from(!image.system_streams.is_empty());
    let node_count = image
        .entries
        .len()
        .checked_add(extra_nodes)
        .and_then(|n| n.checked_add(1))
        .context("entry count overflow")?;
    ensure!(
        node_count <= options.max_entries,
        "writer entry limit exceeded"
    );
    ensure!(
        node_count as u64 * std::mem::size_of::<Node>() as u64 <= options.max_metadata_bytes,
        "writer metadata limit exceeded"
    );
    ensure!(
        image.entries.len() < options.max_entries,
        "writer entry limit exceeded"
    );
    ensure!(
        options.revision.number() >= 0x200
            || (image.system_streams.is_empty()
                && image.root_streams.is_empty()
                && image.entries.values().all(|e| e.streams.is_empty())),
        "named/system streams require UDF 2.00 or newer"
    );
    let mut nodes = vec![node(String::new(), Vec::new(), 0, Payload::Directory)];
    let mut names = BTreeMap::from([(String::new(), 0usize)]);
    for (path, input) in &image.entries {
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        let parent = *names.get(parent).context("missing directory parent")?;
        let name = compressed(path.rsplit('/').next().context("missing name")?)?;
        let mut n = node(path.clone(), name, parent, input.payload.clone());
        n.preallocated = input.preallocated;
        let idx = nodes.len();
        nodes.push(n);
        nodes[parent].children.push(idx);
        names.insert(path.clone(), idx);
    }
    for i in 1..nodes.len() {
        if let Payload::HardLink(target) = &nodes[i].payload {
            let target = *names
                .get(target)
                .context("hard link target does not exist")?;
            ensure!(
                matches!(nodes[target].payload, Payload::Pieces(_)),
                "hard link target must be a regular file"
            );
            nodes[i].alias = Some(target);
        }
    }
    for (path, input) in &image.entries {
        if !input.streams.is_empty() {
            let owner = names[path];
            let idx = nodes.len();
            let mut n = node(String::new(), Vec::new(), owner, Payload::Directory);
            n.system = true;
            n.stream_owner = Some(owner);
            nodes.push(n);
            nodes[owner].stream_dir = Some(idx);
            for (name, bytes) in &input.streams {
                let child = nodes.len();
                nodes.push(node(String::new(), compressed(name)?, idx, bytes.clone()));
                nodes[child].stream_owner = Some(owner);
                nodes[idx].children.push(child);
            }
        }
    }
    if !image.root_streams.is_empty() {
        let index = nodes.len();
        let mut directory = node(String::new(), Vec::new(), 0, Payload::Directory);
        directory.system = true;
        directory.stream_owner = Some(0);
        nodes.push(directory);
        nodes[0].stream_dir = Some(index);
        for (name, bytes) in &image.root_streams {
            let child = nodes.len();
            let mut entry = node(String::new(), compressed(name)?, index, bytes.clone());
            entry.stream_owner = Some(0);
            nodes.push(entry);
            nodes[index].children.push(child);
        }
    }
    let system = if image.system_streams.is_empty() {
        None
    } else {
        let idx = nodes.len();
        let mut n = node(String::new(), Vec::new(), idx, Payload::Directory);
        n.system = true;
        n.system_stream = true;
        nodes.push(n);
        for (name, bytes) in &image.system_streams {
            let child = nodes.len();
            nodes.push(node(String::new(), compressed(name)?, idx, bytes.clone()));
            nodes[child].system_stream = true;
            nodes[idx].children.push(child);
        }
        Some(idx)
    };
    ensure!(
        nodes.len() <= options.max_entries,
        "writer entry limit exceeded"
    );
    let mut links = vec![0u32; nodes.len()];
    for directory in nodes.iter().filter(|n| n.directory()) {
        let parent = nodes[directory.parent].alias.unwrap_or(directory.parent);
        links[parent] = links[parent]
            .checked_add(1)
            .context("file link count overflow")?;
        for child in &directory.children {
            let target = nodes[*child].alias.unwrap_or(*child);
            links[target] = links[target]
                .checked_add(1)
                .context("file link count overflow")?;
        }
    }
    for (node, links) in nodes.iter_mut().zip(links) {
        node.links = u16::try_from(links).context("too many file links")?;
    }
    Ok((nodes, system))
}

#[path = "udf_writer_volume.rs"]
mod volume;
fn ad_size(mode: AllocationMode) -> usize {
    match mode {
        AllocationMode::Short => 8,
        AllocationMode::Long => 16,
        AllocationMode::Extended => 20,
        AllocationMode::Embedded => 0,
    }
}
fn encode_ad(bytes: &mut [u8], position: usize, ad: Ad, mode: AllocationMode) {
    put32(bytes, position, (ad.kind << 30) | ad.length);
    match mode {
        AllocationMode::Short => put32(bytes, position + 4, ad.block),
        AllocationMode::Long => {
            put32(bytes, position + 4, ad.block);
            put16(bytes, position + 8, ad.partition);
        }
        AllocationMode::Extended => {
            put32(
                bytes,
                position + 4,
                if ad.kind == 0 { ad.length } else { 0 },
            );
            put32(bytes, position + 8, ad.information);
            put32(bytes, position + 12, ad.block);
            put16(bytes, position + 16, ad.partition);
        }
        AllocationMode::Embedded => {}
    }
}
fn add_extent(
    allocations: &mut Vec<Ad>,
    kind: u32,
    mut length: u64,
    mut block: u32,
    partition: u16,
    options: &UdfOptions,
    budget: &mut u64,
) -> Result<()> {
    let maximum = u64::from(options.extent_blocks) * BLOCK;
    while length > 0 {
        *budget = budget
            .checked_add(std::mem::size_of::<Ad>() as u64)
            .context("allocation metadata overflow")?;
        ensure!(
            *budget <= options.max_metadata_bytes,
            "allocation metadata limit exceeded"
        );
        ensure!(
            (allocations.len() as u64 + 1) * std::mem::size_of::<Ad>() as u64
                <= options.max_metadata_bytes,
            "allocation metadata limit exceeded"
        );
        let count = length.min(maximum);
        allocations.push(Ad {
            kind,
            length: u32::try_from(count)?,
            information: u32::try_from(count)?,
            block,
            partition,
        });
        length -= count;
        if kind != 2 {
            block = block
                .checked_add(blocks(count)?)
                .context("extent block overflow")?;
        }
    }
    Ok(())
}
fn payload_size(payload: &Payload) -> Result<u64> {
    match payload {
        Payload::Pieces(pieces) => pieces.iter().try_fold(0u64, |sum, p| {
            sum.checked_add(match p {
                Piece::Deferred(content) => content.len(),
                Piece::Hole(n) | Piece::AllocatedHole(n) => *n,
            })
            .context("file size overflow")
        }),
        Payload::Link(target) => Ok(link_bytes(target)?.len() as u64),
        _ => Ok(0),
    }
}
fn small_payload(payload: &Payload, size: u64) -> Result<Vec<u8>> {
    let mut result = Vec::with_capacity(usize::try_from(size)?);
    match payload {
        Payload::Pieces(pieces) => {
            for p in pieces.iter() {
                match p {
                    Piece::Deferred(content) => {
                        let start = result.len();
                        result.resize(start + usize::try_from(content.len())?, 0);
                        content.read_exact_at(0, &mut result[start..])?;
                    }
                    Piece::Hole(n) | Piece::AllocatedHole(n) => {
                        result.resize(result.len() + usize::try_from(*n)?, 0)
                    }
                }
            }
        }
        Payload::Link(target) => result = link_bytes(target)?,
        _ => {}
    }
    Ok(result)
}
fn unique_id(nodes: &[Node], index: usize) -> u64 {
    let node = &nodes[index];
    if node.system_stream {
        0
    } else if let Some(owner) = node.stream_owner {
        if owner == 0 { 0 } else { owner as u64 + 15 }
    } else if index == 0 {
        0
    } else {
        index as u64 + 15
    }
}
fn directory_bytes(
    nodes: &[Node],
    index: usize,
    namespace: u16,
    options: &UdfOptions,
) -> Result<Vec<u8>> {
    let node = &nodes[index];
    let mut result = Vec::new();
    let records = std::iter::once((node.parent, &[][..], nodes[node.parent].directory(), true))
        .chain(node.children.iter().map(|child| {
            (
                *child,
                nodes[*child].name.as_slice(),
                nodes[*child].directory(),
                false,
            )
        }));
    for (child, name, directory, parent) in records {
        let len = (38 + name.len() + 3) & !3;
        let remaining = (2048 - (result.len() + len) % 2048) % 2048;
        let implementation = if remaining > 0 && remaining < 16 {
            remaining + 32
        } else {
            0
        };
        let len = (38 + implementation + name.len() + 3) & !3;
        ensure!(
            result.len() as u64 + len as u64 <= options.max_metadata_bytes,
            "directory metadata limit exceeded"
        );
        let mut b = vec![0; len];
        put16(&mut b, 16, 1);
        b[18] = if directory { 2 } else { 0 } | if parent { 8 } else { 0 };
        b[19] = name.len() as u8;
        let target = nodes[child].alias.unwrap_or(child);
        let n = &nodes[target];
        let extent = if options.icb_strategy == IcbStrategy::Strategy4096 {
            4096
        } else {
            2048
        };
        long_ad(&mut b, 20, extent, n.icb, namespace);
        // FID implementation-use UniqueID identifies the same hard-linked object.
        put32(
            &mut b,
            32,
            u32::try_from(unique_id(nodes, if parent { target } else { child }))?,
        );
        put16(&mut b, 36, implementation as u16);
        if implementation > 0 {
            reg(&mut b, 38, b"*libmkiso", None);
        }
        b[38 + implementation..38 + implementation + name.len()].copy_from_slice(name);
        let block = node
            .data
            .checked_add(u32::try_from(result.len() / 2048)?)
            .context("directory block overflow")?;
        tag(&mut b, 257, block, len, options.revision);
        result.extend(b);
    }
    Ok(result)
}
type AllocationLists = (Vec<Ad>, Vec<(u32, Vec<Ad>)>);
type EntryEncoding = ([u8; 2048], Vec<(u32, [u8; 2048])>);
fn allocation_lists(node: &Node, header: usize, namespace: u16) -> Result<AllocationLists> {
    let size = ad_size(node.mode);
    if size == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let first_capacity = (2048 - header) / size;
    let aed_capacity = (2048 - 24) / size;
    let mut pos = 0;
    let mut first = Vec::new();
    let count = if node.aeds.is_empty() {
        node.allocations.len()
    } else {
        first_capacity - 1
    };
    first.extend_from_slice(&node.allocations[..count]);
    pos += count;
    if let Some(next) = node.aeds.first() {
        first.push(Ad {
            kind: 3,
            length: 2048,
            information: 0,
            block: *next,
            partition: namespace,
        });
    }
    let mut continuation = Vec::new();
    for (index, block) in node.aeds.iter().enumerate() {
        let remaining = node.allocations.len() - pos;
        let count = if index + 1 < node.aeds.len() {
            aed_capacity - 1
        } else {
            remaining
        };
        let mut ads = node.allocations[pos..pos + count].to_vec();
        pos += count;
        if let Some(next) = node.aeds.get(index + 1) {
            ads.push(Ad {
                kind: 3,
                length: 2048,
                information: 0,
                block: *next,
                partition: namespace,
            });
        }
        continuation.push((*block, ads));
    }
    ensure!(
        pos == node.allocations.len(),
        "allocation continuation layout mismatch"
    );
    Ok((first, continuation))
}
fn entry_bytes(
    nodes: &[Node],
    index: usize,
    namespace: u16,
    options: &UdfOptions,
) -> Result<EntryEncoding> {
    let n = &nodes[index];
    let efe = options.revision.number() >= 0x200;
    let header = if efe { 216 } else { 176 };
    let mut b = [0; 2048];
    put16(
        &mut b,
        20,
        if options.icb_strategy == IcbStrategy::Strategy4096 {
            4096
        } else {
            4
        },
    );
    put16(&mut b, 24, 1);
    b[27] = n.file_type();
    put16(
        &mut b,
        34,
        match n.mode {
            AllocationMode::Short => 0,
            AllocationMode::Long => 1,
            AllocationMode::Extended => 2,
            AllocationMode::Embedded => 3,
        } | if !n.directory() && (n.stream_owner.is_some() || n.system_stream) {
            1 << 13
        } else {
            0
        },
    );
    put32(&mut b, 36, u32::MAX);
    put32(&mut b, 40, u32::MAX);
    put32(&mut b, 44, 0x14a5);
    put16(&mut b, 48, n.links);
    put64(&mut b, 56, n.size);
    let recorded = n
        .allocations
        .iter()
        .filter(|a| a.kind == 0)
        .map(|a| u64::from(a.length).div_ceil(BLOCK))
        .sum();
    if efe {
        let stream_size = if let Some(dir) = n.stream_dir {
            nodes[dir].children.iter().try_fold(0u64, |sum, child| {
                sum.checked_add(nodes[*child].size)
                    .context("stream object size overflow")
            })?
        } else {
            0
        };
        put64(
            &mut b,
            64,
            n.size
                .checked_add(stream_size)
                .context("object size overflow")?,
        );
        put64(&mut b, 72, recorded);
        for p in [80, 92, 104, 116] {
            timestamp(&mut b, p, options.timestamp);
        }
        put32(&mut b, 128, 1);
        reg(&mut b, 168, b"*libmkiso", None);
        put64(&mut b, 200, unique_id(nodes, index));
        if let Some(dir) = n.stream_dir {
            long_ad(
                &mut b,
                152,
                if options.icb_strategy == IcbStrategy::Strategy4096 {
                    4096
                } else {
                    2048
                },
                nodes[dir].icb,
                namespace,
            );
        }
    } else {
        put64(&mut b, 64, recorded);
        for p in [72, 84, 96] {
            timestamp(&mut b, p, options.timestamp);
        }
        put32(&mut b, 108, 1);
        reg(&mut b, 128, b"*libmkiso", None);
        put64(&mut b, 160, unique_id(nodes, index));
    }
    let (initial, continuations) = allocation_lists(n, header, namespace)?;
    let len = if n.mode == AllocationMode::Embedded {
        b[header..header + n.embedded.len()].copy_from_slice(&n.embedded);
        n.embedded.len()
    } else {
        for (i, ad) in initial.iter().enumerate() {
            encode_ad(&mut b, header + i * ad_size(n.mode), *ad, n.mode);
        }
        initial.len() * ad_size(n.mode)
    };
    put32(&mut b, header - 4, len as u32);
    tag(
        &mut b,
        if efe { 266 } else { 261 },
        n.entry,
        header + len,
        options.revision,
    );
    let mut aeds = Vec::new();
    for (block, ads) in continuations {
        let mut d = [0; 2048];
        put32(&mut d, 20, (ads.len() * ad_size(n.mode)) as u32);
        for (i, ad) in ads.iter().enumerate() {
            encode_ad(&mut d, 24 + i * ad_size(n.mode), *ad, n.mode);
        }
        tag(
            &mut d,
            258,
            block,
            24 + ads.len() * ad_size(n.mode),
            options.revision,
        );
        aeds.push((block, d));
    }
    Ok((b, aeds))
}
fn sector(output: &mut File, physical: u32, bytes: &[u8]) -> Result<()> {
    output.seek(SeekFrom::Start(u64::from(physical) * BLOCK))?;
    output.write_all(bytes)?;
    Ok(())
}
fn write_payload(
    output: &mut File,
    node: &Node,
    physical: u32,
    checkpoint: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    output.seek(SeekFrom::Start(u64::from(physical) * BLOCK))?;
    match &node.payload {
        Payload::Pieces(pieces) => {
            let mut position = u64::from(physical) * BLOCK;
            for p in pieces.iter() {
                if let Piece::Deferred(content) = p {
                    let mut copied = 0;
                    let mut buffer = [0; 65536];
                    output.seek(SeekFrom::Start(position))?;
                    content.validate()?;
                    while copied < content.len() {
                        checkpoint()?;
                        let count =
                            usize::try_from((content.len() - copied).min(buffer.len() as u64))?;
                        content.read_exact_at(copied, &mut buffer[..count])?;
                        output.write_all(&buffer[..count])?;
                        copied += count as u64;
                    }
                    content.validate()?;
                    position = position
                        .checked_add(u64::from(blocks(content.len())?) * BLOCK)
                        .context("payload offset overflow")?;
                } else if let Piece::AllocatedHole(length) = p {
                    position = position
                        .checked_add(u64::from(blocks(*length)?) * BLOCK)
                        .context("payload offset overflow")?;
                }
            }
        }
        Payload::Link(target) => output.write_all(&link_bytes(target)?)?,
        _ => {}
    }
    Ok(())
}

fn metadata_span(blocks: u32, extent_blocks: u32) -> Result<u32> {
    if extent_blocks == 0 {
        return Ok(blocks);
    }
    blocks
        .checked_add(
            blocks
                .div_ceil(extent_blocks)
                .saturating_sub(1)
                .checked_mul(32)
                .context("metadata gap overflow")?,
        )
        .context("metadata span overflow")
}
fn write_namespace(
    output: &mut File,
    layout: &volume::VolumeLayout,
    block: u32,
    data: &[u8],
) -> Result<()> {
    if let Some(metadata) = &layout.metadata {
        for (index, bytes) in data.chunks(BLOCK as usize).enumerate() {
            let logical = block
                .checked_add(u32::try_from(index)?)
                .context("namespace offset overflow")?;
            ensure!(
                logical < metadata.blocks,
                "namespace exceeds metadata partition"
            );
            let gap = logical
                .checked_div(metadata.extent_blocks)
                .unwrap_or(0)
                .checked_mul(32)
                .context("metadata gap overflow")?;
            let physical = PART
                .checked_add(metadata.primary_data)
                .and_then(|start| start.checked_add(logical))
                .and_then(|start| start.checked_add(gap))
                .context("metadata address overflow")?;
            sector(output, physical, bytes)?;
        }
        Ok(())
    } else {
        sector(
            output,
            PART.checked_add(block)
                .context("namespace address overflow")?,
            data,
        )
    }
}

fn write_image(
    image: &UdfImage,
    output: &Path,
    options: &UdfOptions,
    checkpoint: &mut impl FnMut() -> Result<()>,
    staged: bool,
) -> Result<String> {
    checkpoint()?;
    options.validate()?;
    ensure!(staged || !output.exists(), "output already exists");
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let mut initial_metadata = 0u64;
    for (path, input) in &image.entries {
        let name = path.rsplit('/').next().context("missing UDF name")?;
        initial_metadata = initial_metadata
            .checked_add(path.len() as u64 + compressed(name)?.len() as u64)
            .context("metadata size overflow")?;
        let target_bytes = match &input.payload {
            Payload::Link(target) => target.len(),
            Payload::HardLink(target) => target.len(),
            _ => 0,
        };
        initial_metadata = initial_metadata
            .checked_add(target_bytes as u64)
            .context("metadata size overflow")?;
        for name in input.streams.keys() {
            initial_metadata = initial_metadata
                .checked_add(compressed(name)?.len() as u64)
                .context("metadata size overflow")?;
        }
    }
    for name in image.root_streams.keys().chain(image.system_streams.keys()) {
        initial_metadata = initial_metadata
            .checked_add(compressed(name)?.len() as u64)
            .context("metadata size overflow")?;
    }
    ensure!(
        initial_metadata <= options.max_metadata_bytes,
        "writer metadata limit exceeded"
    );
    for path in image.entries.keys() {
        ensure!(
            path.split('/').count() <= options.max_nesting_depth,
            "writer nesting limit exceeded"
        );
    }
    let (mut nodes, system) = build_nodes(image, options)?;
    let mut boot_nodes = Vec::new();
    for boot in options.boot.bios.iter().chain(options.boot.efi.iter()) {
        let path = host_relative_path(&boot.path)?;
        let index = nodes
            .iter()
            .position(|node| node.path == path)
            .context("boot image is missing from UDF tree")?;
        let target = nodes[index].alias.unwrap_or(index);
        let size = payload_size(&nodes[target].payload)?;
        boot.validate(u32::try_from(size).context("boot image exceeds 32-bit byte length")?)?;
        ensure!(
            matches!(&nodes[target].payload, Payload::Pieces(pieces) if pieces.iter().all(|piece|matches!(piece,Piece::Deferred(_)))),
            "boot image must be a recorded regular file"
        );
        boot_nodes.push(target);
    }
    let mut metadata = initial_metadata
        .checked_add(nodes.len() as u64 * std::mem::size_of::<Node>() as u64)
        .context("metadata size overflow")?;
    ensure!(
        metadata <= options.max_metadata_bytes,
        "writer metadata limit exceeded"
    );
    let is_metadata = matches!(
        options.partition,
        UdfPartition::Metadata { .. } | UdfPartition::MetadataSparable { .. }
    );
    let virtual_map = options.partition == UdfPartition::Virtual;
    let namespace = if is_metadata || virtual_map { 1 } else { 0 };
    let header = if options.revision.number() >= 0x200 {
        216
    } else {
        176
    };
    let mut cursor = u32::from(options.file_set_descriptors);
    for n in &mut nodes {
        checkpoint()?;
        if n.alias.is_some() {
            continue;
        }
        let count = if options.icb_strategy == IcbStrategy::Direct {
            1
        } else {
            2
        };
        n.icb = take(&mut cursor, count)?;
        n.entry = n.icb + u32::from(options.icb_strategy == IcbStrategy::Indirect);
    }
    for i in 0..nodes.len() {
        if let Some(target) = nodes[i].alias {
            nodes[i].icb = nodes[target].icb;
            nodes[i].entry = nodes[target].entry;
        }
    }
    for i in 0..nodes.len() {
        checkpoint()?;
        if nodes[i].directory() {
            nodes[i].data = cursor;
            let bytes = directory_bytes(&nodes, i, namespace, options)?;
            nodes[i].size = bytes.len() as u64;
            metadata = metadata
                .checked_add(nodes[i].size)
                .context("metadata size overflow")?;
            ensure!(
                metadata <= options.max_metadata_bytes,
                "writer metadata limit exceeded"
            );
            take(&mut cursor, blocks(nodes[i].size)?)?;
            nodes[i].mode = if virtual_map {
                AllocationMode::Long
            } else {
                AllocationMode::Short
            };
            let data = nodes[i].data;
            let size = nodes[i].size;
            add_extent(
                &mut nodes[i].allocations,
                0,
                size,
                data,
                if virtual_map { 0 } else { namespace },
                options,
                &mut metadata,
            )?;
        }
    }
    let payload_start = cursor;
    let mut physical_cursor = if is_metadata { 0 } else { cursor };
    let mut total_bytes = 0u64;
    let mut payload_order: Vec<_> = (0..nodes.len())
        .filter(|index| !nodes[*index].directory() && nodes[*index].alias.is_none())
        .collect();
    payload_order.sort_by_key(|index| !boot_nodes.contains(index));
    for index in payload_order {
        let node = &mut nodes[index];
        checkpoint()?;
        if node.directory() || node.alias.is_some() {
            continue;
        }
        let size = payload_size(&node.payload)?;
        ensure!(
            size <= options.max_entry_bytes,
            "writer entry byte limit exceeded"
        );
        total_bytes = total_bytes
            .checked_add(size)
            .context("total size overflow")?;
        ensure!(
            total_bytes <= options.max_total_bytes,
            "writer total byte limit exceeded"
        );
        node.size = size;
        let boot = boot_nodes.contains(&index);
        if boot {
            ensure!(
                matches!(&node.payload,Payload::Pieces(p) if p.iter().all(|p|matches!(p,Piece::Deferred(_)))),
                "boot image must be recorded and contiguous"
            );
        }
        let can_embed = options.allocation == AllocationMode::Embedded
            && size <= (2048 - header) as u64
            && node.preallocated == 0
            && !boot
            && !matches!(&node.payload, Payload::Pieces(pieces) if pieces.iter().any(|piece|!matches!(piece,Piece::Deferred(_))));
        if can_embed {
            node.mode = AllocationMode::Embedded;
            node.embedded = small_payload(&node.payload, size)?;
            metadata = metadata
                .checked_add(size)
                .context("metadata size overflow")?;
            ensure!(
                metadata <= options.max_metadata_bytes,
                "writer metadata limit exceeded"
            );
            continue;
        }
        node.mode = match options.allocation {
            AllocationMode::Embedded => {
                if is_metadata || virtual_map || options.partition == UdfPartition::PhysicalSplit {
                    AllocationMode::Long
                } else {
                    AllocationMode::Short
                }
            }
            mode => mode,
        };
        node.data = physical_cursor;
        match node.payload.clone() {
            Payload::Link(target) => {
                let size = link_bytes(&target)?.len() as u64;
                let start = take(&mut physical_cursor, blocks(size)?)?;
                add_extent(
                    &mut node.allocations,
                    0,
                    size,
                    start,
                    0,
                    options,
                    &mut metadata,
                )?;
            }
            Payload::Pieces(pieces) => {
                let nonempty: Vec<_> = pieces
                    .iter()
                    .filter(|p| match p {
                        Piece::Deferred(content) => !content.is_empty(),
                        Piece::Hole(n) | Piece::AllocatedHole(n) => *n > 0,
                    })
                    .collect();
                for (index, p) in nonempty.iter().enumerate() {
                    let (kind, length) = match p {
                        Piece::Deferred(content) => (0, content.len()),
                        Piece::Hole(n) => (2, *n),
                        Piece::AllocatedHole(n) => (1, *n),
                    };
                    ensure!(
                        index + 1 == nonempty.len() || length.is_multiple_of(BLOCK),
                        "interior sparse segments must be block aligned"
                    );
                    let block = if kind != 2 {
                        take(&mut physical_cursor, blocks(length)?)?
                    } else {
                        0
                    };
                    add_extent(
                        &mut node.allocations,
                        kind,
                        length,
                        block,
                        0,
                        options,
                        &mut metadata,
                    )?;
                }
            }
            _ => {}
        }
        if node.preallocated > 0 {
            let length = u64::from(node.preallocated) * BLOCK;
            let start = take(&mut physical_cursor, node.preallocated)?;
            let first_tail = node.allocations.len();
            add_extent(
                &mut node.allocations,
                1,
                length,
                start,
                0,
                options,
                &mut metadata,
            )?;
            for tail in &mut node.allocations[first_tail..] {
                tail.information = 0;
            }
        }
    }
    for i in 0..nodes.len() {
        if let Some(target) = nodes[i].alias {
            nodes[i].size = nodes[target].size;
            total_bytes = total_bytes
                .checked_add(nodes[i].size)
                .context("total size overflow")?;
            ensure!(
                total_bytes <= options.max_total_bytes,
                "writer total byte limit exceeded"
            );
        }
    }
    if !is_metadata {
        cursor = physical_cursor;
    }
    for n in &mut nodes {
        if n.alias.is_some() || n.mode == AllocationMode::Embedded {
            continue;
        }
        let capacity = (2048 - header) / ad_size(n.mode);
        let continuation = (2048 - 24) / ad_size(n.mode) - 1;
        let count = if n.allocations.len() > capacity {
            (n.allocations.len() - (capacity - 1)).div_ceil(continuation)
        } else {
            0
        };
        for _ in 0..count {
            n.aeds.push(take(&mut cursor, 1)?);
        }
        metadata = metadata
            .checked_add((n.aeds.len() * 2048) as u64)
            .context("metadata size overflow")?;
        ensure!(
            metadata <= options.max_metadata_bytes,
            "writer metadata limit exceeded"
        );
    }
    let mut payload_partition_start = None;
    if options.partition == UdfPartition::PhysicalSplit {
        let aed_count = u32::try_from(nodes.iter().map(|node| node.aeds.len()).sum::<usize>())?;
        let split = payload_start
            .checked_add(aed_count)
            .context("split partition address overflow")?;
        let mut aed_cursor = payload_start;
        for node in &mut nodes {
            for block in &mut node.aeds {
                *block = take(&mut aed_cursor, 1)?;
            }
            if !node.directory() && node.alias.is_none() && node.mode != AllocationMode::Embedded {
                node.data = node
                    .data
                    .checked_add(aed_count)
                    .context("split payload overflow")?;
                for ad in &mut node.allocations {
                    if ad.kind != 2 {
                        ad.block = ad
                            .block
                            .checked_sub(payload_start)
                            .context("split allocation underflow")?;
                    }
                    ad.partition = 1;
                }
            }
        }
        physical_cursor = cursor.max(split.checked_add(1).context("split partition overflow")?);
        payload_partition_start = Some(split);
    }
    let mut metadata_layout = None;
    if let UdfPartition::Metadata { mirror } | UdfPartition::MetadataSparable { mirror, .. } =
        options.partition
    {
        let count = cursor
            .div_ceil(32)
            .checked_mul(32)
            .context("metadata layout overflow")?;
        let writable_metadata = matches!(options.partition, UdfPartition::MetadataSparable { .. });
        let metadata_bitmap_blocks = blocks(24 + u64::from(count).div_ceil(8))?;
        let namespace_base = if writable_metadata {
            3u32.checked_add(metadata_bitmap_blocks)
                .context("metadata bitmap overflow")?
                .div_ceil(32)
                .checked_mul(32)
                .context("metadata bitmap overflow")?
        } else {
            32u32
        };
        let span = metadata_span(count, options.metadata_extent_blocks)?;
        let mirror_data = if mirror {
            namespace_base
                .checked_add(span)
                .context("metadata layout overflow")?
        } else {
            namespace_base
        };
        let payload_base = namespace_base
            .checked_add(
                span.checked_mul(if mirror { 2 } else { 1 })
                    .context("metadata layout overflow")?,
            )
            .context("metadata layout overflow")?;
        for n in &mut nodes {
            if !n.directory() && n.alias.is_none() && n.mode != AllocationMode::Embedded {
                n.data = n
                    .data
                    .checked_add(payload_base)
                    .context("payload address overflow")?;
                for ad in &mut n.allocations {
                    if ad.kind != 2 {
                        ad.block = ad
                            .block
                            .checked_add(payload_base)
                            .context("payload address overflow")?;
                    }
                }
            }
        }
        physical_cursor = physical_cursor
            .checked_add(payload_base)
            .context("partition length overflow")?;
        metadata_layout = Some(volume::MetadataLayout {
            primary_icb: 0,
            mirror_icb: 1,
            primary_data: namespace_base,
            mirror_data,
            blocks: count,
            duplicated: mirror,
            extent_blocks: options.metadata_extent_blocks,
            bitmap_icb: writable_metadata.then_some(2),
            bitmap_data: if writable_metadata { 3 } else { 0 },
        });
    } else if payload_partition_start.is_none() {
        physical_cursor = cursor;
    }
    let mut vat_layout = None;
    let mut bitmap = None;
    let mut bitmap_blocks = 0;
    if virtual_map {
        let vat_entries = physical_cursor;
        let length = if options.revision == UdfRevision::V150 {
            36
        } else {
            152
        } + u64::from(vat_entries) * 4;
        metadata = metadata
            .checked_add(length)
            .context("VAT metadata overflow")?;
        ensure!(
            metadata <= options.max_metadata_bytes,
            "VAT metadata limit exceeded"
        );
        let data = take(&mut physical_cursor, blocks(length)?)?;
        vat_layout = Some(volume::VatLayout {
            entries: vat_entries,
            data,
            icb: 0,
            mapped_start: 0,
        });
    }
    if let UdfPartition::Sparable { packet_blocks }
    | UdfPartition::MetadataSparable { packet_blocks, .. } = options.partition
    {
        let packet = u32::from(packet_blocks);
        physical_cursor = physical_cursor
            .div_ceil(packet)
            .checked_mul(packet)
            .context("partition alignment overflow")?;
        bitmap = Some(physical_cursor);
        bitmap_blocks = blocks(24 + u64::from(physical_cursor).div_ceil(8))?;
        let final_blocks = physical_cursor
            .checked_add(bitmap_blocks)
            .context("bitmap layout overflow")?
            .div_ceil(packet)
            .checked_mul(packet)
            .context("partition alignment overflow")?;
        bitmap_blocks = blocks(24 + u64::from(final_blocks).div_ceil(8))?;
        physical_cursor = physical_cursor
            .checked_add(bitmap_blocks)
            .context("bitmap layout overflow")?
            .div_ceil(packet)
            .checked_mul(packet)
            .context("partition alignment overflow")?;
    }
    let reserve = PART
        .checked_add(physical_cursor)
        .context("volume layout overflow")?;
    let mut total = reserve.checked_add(273).context("volume layout overflow")?;
    let partition_blocks = if let Some(vat) = vat_layout.as_mut() {
        vat.icb = total - PART - 1;
        total - PART
    } else {
        physical_cursor
    };
    let mut spare_packets = Vec::new();
    if !options.sparing_packets.is_empty() {
        let packet = match options.partition {
            UdfPartition::Sparable { packet_blocks }
            | UdfPartition::MetadataSparable { packet_blocks, .. } => u32::from(packet_blocks),
            _ => unreachable!("validated sparable profile"),
        };
        let table_blocks = blocks(56 + options.sparing_packets.len() as u64 * 8)?;
        let second_table = PART
            .checked_add(partition_blocks)
            .context("sparing location overflow")?
            .div_ceil(32)
            .checked_mul(32)
            .and_then(|block| block.checked_add(32))
            .context("sparing location overflow")?;
        let mut replacement = second_table
            .checked_add(table_blocks)
            .context("sparing location overflow")?
            .div_ceil(packet)
            .checked_mul(packet)
            .context("sparing alignment overflow")?;
        let mut requested = options.sparing_packets.clone();
        requested.sort_unstable();
        ensure!(
            requested.windows(2).all(|pair| pair[0] != pair[1]),
            "duplicate sparing packet"
        );
        for original in requested {
            ensure!(
                original.is_multiple_of(packet)
                    && original
                        .checked_add(packet)
                        .is_some_and(|end| end <= partition_blocks),
                "sparing packet outside partition or unaligned"
            );
            spare_packets.push((original, replacement));
            replacement = replacement
                .checked_add(packet)
                .context("replacement area overflow")?;
        }
        total = total.max(
            replacement
                .checked_add(257)
                .context("sparing backup anchor overflow")?,
        );
        metadata = metadata
            .checked_add(options.sparing_packets.len() as u64 * 16)
            .context("sparing metadata overflow")?;
        ensure!(
            metadata <= options.max_metadata_bytes,
            "sparing metadata limit exceeded"
        );
    }
    ensure!(
        u64::from(total) * BLOCK <= options.max_image_bytes,
        "writer image byte limit exceeded"
    );
    let layout = volume::VolumeLayout {
        partition_blocks,
        files: u32::try_from(
            image
                .entries
                .values()
                .filter(|input| !matches!(input.payload, Payload::Directory))
                .count(),
        )?,
        directories: u32::try_from(
            1 + image
                .entries
                .values()
                .filter(|input| matches!(input.payload, Payload::Directory))
                .count(),
        )?,
        namespace_partition: namespace,
        metadata: metadata_layout,
        vat: vat_layout,
        bitmap,
        bitmap_blocks,
        payload_partition_start,
        spare_packets,
    };
    let (mut file, temp) = tempfile::NamedTempFile::new_in(&parent)?.into_parts();
    file.set_len(u64::from(total) * BLOCK)?;
    for index in 0..u32::from(options.file_set_descriptors) {
        let mut b = [0; 2048];
        timestamp(&mut b, 16, options.timestamp);
        put16(&mut b, 28, 3);
        put16(&mut b, 30, 3);
        put32(&mut b, 32, 1);
        put32(&mut b, 36, 1);
        put32(&mut b, 40, index);
        chars(&mut b, 48);
        dstring(&mut b, 112, 128, &options.label)?;
        chars(&mut b, 240);
        dstring(&mut b, 304, 32, &options.label)?;
        long_ad(
            &mut b,
            400,
            if options.icb_strategy == IcbStrategy::Strategy4096 {
                4096
            } else {
                2048
            },
            nodes[0].icb,
            namespace,
        );
        reg(&mut b, 416, b"*OSTA UDF Compliant", Some(options.revision));
        if options.file_set_descriptors > 1 {
            b[442] = 1;
        }
        if index + 1 < u32::from(options.file_set_descriptors) {
            long_ad(&mut b, 448, 2048, index + 1, namespace);
        }
        if let Some(system) = system {
            long_ad(
                &mut b,
                464,
                if options.icb_strategy == IcbStrategy::Strategy4096 {
                    4096
                } else {
                    2048
                },
                nodes[system].icb,
                namespace,
            );
        }
        tag(&mut b, 256, index, 512, options.revision);
        write_namespace(&mut file, &layout, index, &b)?;
    }
    for i in 0..nodes.len() {
        checkpoint()?;
        let n = &nodes[i];
        if n.alias.is_some() {
            continue;
        }
        let (b, aeds) = entry_bytes(&nodes, i, namespace, options)?;
        write_namespace(&mut file, &layout, n.entry, &b)?;
        for (block, b) in aeds {
            write_namespace(&mut file, &layout, block, &b)?;
        }
        match options.icb_strategy {
            IcbStrategy::Indirect => {
                let mut b = [0; 2048];
                put16(&mut b, 20, 4);
                put16(&mut b, 24, 1);
                b[27] = 3;
                long_ad(&mut b, 36, 2048, n.entry, namespace);
                tag(&mut b, 259, n.icb, 52, options.revision);
                write_namespace(&mut file, &layout, n.icb, &b)?;
            }
            IcbStrategy::Strategy4096 => {
                let mut b = [0; 2048];
                put16(&mut b, 20, 4096);
                put16(&mut b, 24, 1);
                b[27] = 11;
                tag(&mut b, 260, n.icb + 1, 36, options.revision);
                write_namespace(&mut file, &layout, n.icb + 1, &b)?;
            }
            IcbStrategy::Direct => {}
        }
        if n.directory() {
            write_namespace(
                &mut file,
                &layout,
                n.data,
                &directory_bytes(&nodes, i, namespace, options)?,
            )?;
        } else if n.mode != AllocationMode::Embedded {
            write_payload(&mut file, n, PART + n.data, checkpoint)?;
        }
    }
    if let Some(metadata) = &layout.metadata
        && metadata.duplicated
    {
        let mut buffer = [0; 65536];
        let from = u64::from(PART + metadata.primary_data) * BLOCK;
        let to = u64::from(PART + metadata.mirror_data) * BLOCK;
        let length = u64::from(metadata_span(metadata.blocks, metadata.extent_blocks)?) * BLOCK;
        let mut copied = 0;
        while copied < length {
            checkpoint()?;
            let count = usize::try_from((length - copied).min(buffer.len() as u64))?;
            file.seek(SeekFrom::Start(from + copied))?;
            file.read_exact(&mut buffer[..count])?;
            file.seek(SeekFrom::Start(to + copied))?;
            file.write_all(&buffer[..count])?;
            copied += count as u64;
        }
    }

    volume::emit(&mut file, options, &layout, total, reserve)?;
    if !layout.spare_packets.is_empty() {
        let packet = match options.partition {
            UdfPartition::Sparable { packet_blocks }
            | UdfPartition::MetadataSparable { packet_blocks, .. } => u64::from(packet_blocks),
            _ => unreachable!("validated sparable profile"),
        };
        let mut buffer = [0; 65536];
        for &(original, replacement) in &layout.spare_packets {
            let source = u64::from(
                PART.checked_add(original)
                    .context("sparing source overflow")?,
            ) * BLOCK;
            let target = u64::from(replacement) * BLOCK;
            let mut copied = 0u64;
            while copied < packet * BLOCK {
                checkpoint()?;
                let length = usize::try_from((packet * BLOCK - copied).min(buffer.len() as u64))?;
                file.seek(SeekFrom::Start(source + copied))?;
                file.read_exact(&mut buffer[..length])?;
                file.seek(SeekFrom::Start(target + copied))?;
                file.write_all(&buffer[..length])?;
                copied += length as u64;
            }
        }
    }
    let images: Vec<_> = nodes
        .iter()
        .filter(|n| !n.directory() && !n.path.is_empty())
        .map(|n| {
            let target = n.alias.map_or(n, |i| &nodes[i]);
            (n.path.clone(), PART + target.data, target.size)
        })
        .collect();
    crate::udf_boot::write_bridge(
        &mut file,
        total,
        &options.boot,
        &images,
        options.timestamp,
        &options.label,
    )?;
    file.sync_all()?;
    file.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        checkpoint()?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let hash = hex::encode(digest.finalize());
    drop(file);
    checkpoint()?;
    // A source copied early may drift while later payloads or hashing run.
    // Revalidate every retained snapshot before retaining or publishing output.
    for node in &nodes {
        checkpoint()?;
        if let Payload::Pieces(pieces) = &node.payload {
            for piece in pieces.iter() {
                if let Piece::Deferred(content) = piece {
                    content.validate()?;
                }
            }
        }
    }
    if staged {
        fs::rename(&temp, output).context("retain staged UDF image")?;
    } else {
        fs::hard_link(&temp, output).context("publish UDF image without overwriting output")?;
    }
    Ok(hash)
}
