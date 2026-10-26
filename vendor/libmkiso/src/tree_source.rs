//! Bounded inventory and repeatable deferred payloads for native authoring.
//! Object and stream identifiers are scoped to one source. A source must retain a
//! stable snapshot or report identity, generation or size drift from `validate`.
use std::{fmt, io, sync::Arc};

/// Snapshot identity captured during inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceIdentity {
    pub object: u128,
    pub generation: u64,
    pub size: u64,
}
/// Repeatable positional content. Reads return zero only at EOF; short reads are
/// allowed. Implementations must never read beyond their captured logical size.
pub trait ContentSource {
    fn identity(&self) -> SourceIdentity;
    fn validate(&self, expected: SourceIdentity) -> io::Result<()>;
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize>;
}
/// Retained deferred content and its inventory-time identity.
#[derive(Clone)]
pub struct DeferredContent {
    source: Arc<dyn ContentSource>,
    identity: SourceIdentity,
}
impl fmt::Debug for DeferredContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DeferredContent")
            .field(&self.identity)
            .finish()
    }
}
impl DeferredContent {
    pub fn new(source: Arc<dyn ContentSource>) -> Self {
        let identity = source.identity();
        Self { source, identity }
    }
    pub fn identity(&self) -> SourceIdentity {
        self.identity
    }
    pub fn len(&self) -> u64 {
        self.identity.size
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn validate(&self) -> io::Result<()> {
        self.source.validate(self.identity)
    }
    /// Checked exact read, including repeated reads and validation on each call.
    pub fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        if offset
            .checked_add(buffer.len() as u64)
            .is_none_or(|end| end > self.len())
        {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "content range exceeds snapshot",
            ));
        }
        self.validate()?;
        let mut done = 0;
        while done < buffer.len() {
            let count = self
                .source
                .read_at(offset + done as u64, &mut buffer[done..])?;
            if count == 0 || count > buffer.len() - done {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "invalid deferred short read",
                ));
            }
            done += count;
        }
        self.validate()
    }
}
/// Native name bytes are retained alongside a destination-compatible path.
#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub metadata: crate::preservation::Metadata,
    pub path: String,
    pub native_name: Vec<u8>,
    pub object: u128,
    pub kind: TreeEntryKind,
    pub streams: Vec<TreeStream>,
}
#[derive(Debug, Clone)]
pub enum TreeEntryKind {
    Directory,
    File(Vec<TreeExtent>),
    Symlink(String),
    HardLink(String),
}
#[derive(Debug, Clone)]
pub enum TreeExtent {
    Data(DeferredContent),
    Hole(u64),
    AllocatedHole(u64),
}
#[derive(Debug, Clone)]
pub struct TreeStream {
    pub metadata: crate::preservation::Metadata,
    pub name: String,
    pub native_name: Vec<u8>,
    pub content: DeferredContent,
}
/// Inventory is bounded independently of payload size. This initial contract
/// advertises content preservation only; native metadata requires preflight.
#[derive(Debug, Clone, Default)]
pub struct TreeInventory {
    pub root_metadata: crate::preservation::Metadata,
    pub entries: Vec<TreeEntry>,
    pub root_streams: Vec<TreeStream>,
    pub system_streams: Vec<TreeStream>,
}
impl TreeInventory {
    pub fn validate_budget(&self, maximum_entries: usize, maximum_bytes: usize) -> io::Result<()> {
        let mut count = self.entries.len();
        let mut bytes = 0usize;
        let mut charge = |amount: usize| -> io::Result<()> {
            bytes = bytes
                .checked_add(amount)
                .ok_or_else(|| io::Error::other("inventory overflow"))?;
            if bytes > maximum_bytes {
                return Err(io::Error::other("inventory byte budget exceeded"));
            }
            Ok(())
        };
        charge(self.root_metadata.allocation_bytes())?;
        for entry in &self.entries {
            charge(entry.metadata.allocation_bytes())?;
            charge(std::mem::size_of::<TreeEntry>())?;
            charge(entry.path.len())?;
            charge(entry.native_name.len())?;
            if let TreeEntryKind::File(extents) = &entry.kind {
                charge(
                    extents
                        .len()
                        .checked_mul(std::mem::size_of::<TreeExtent>())
                        .ok_or_else(|| io::Error::other("inventory overflow"))?,
                )?;
            }
            if let TreeEntryKind::Symlink(target) | TreeEntryKind::HardLink(target) = &entry.kind {
                charge(target.len())?;
            }
            count = count
                .checked_add(entry.streams.len())
                .ok_or_else(|| io::Error::other("inventory overflow"))?;
            for stream in &entry.streams {
                charge(
                    std::mem::size_of::<TreeStream>()
                        + stream.name.len()
                        + stream.native_name.len()
                        + stream.metadata.allocation_bytes(),
                )?;
            }
        }
        for stream in self.root_streams.iter().chain(&self.system_streams) {
            count = count
                .checked_add(1)
                .ok_or_else(|| io::Error::other("inventory overflow"))?;
            charge(
                std::mem::size_of::<TreeStream>()
                    + stream.name.len()
                    + stream.native_name.len()
                    + stream.metadata.allocation_bytes(),
            )?;
        }
        if count > maximum_entries {
            return Err(io::Error::other("inventory entry budget exceeded"));
        }
        Ok(())
    }
}
/// Producers perform bounded metadata inspection without loading payloads.
pub trait FileTreeSource {
    fn inventory(&self, maximum_entries: usize, maximum_bytes: usize) -> io::Result<TreeInventory>;
}

/// A bounded logical file assembled from recorded content and zero-filled holes.
#[derive(Debug)]
pub struct ExtentContent {
    identity: SourceIdentity,
    extents: Vec<TreeExtent>,
}
impl ExtentContent {
    pub fn new(object: u128, extents: Vec<TreeExtent>) -> io::Result<Self> {
        let size = extents.iter().try_fold(0u64, |size, extent| {
            size.checked_add(match extent {
                TreeExtent::Data(content) => content.len(),
                TreeExtent::Hole(n) | TreeExtent::AllocatedHole(n) => *n,
            })
            .ok_or_else(|| io::Error::other("logical size overflow"))
        })?;
        Ok(Self {
            identity: SourceIdentity {
                object,
                generation: 0,
                size,
            },
            extents,
        })
    }
}
impl ContentSource for ExtentContent {
    fn identity(&self) -> SourceIdentity {
        self.identity
    }
    fn validate(&self, expected: SourceIdentity) -> io::Result<()> {
        if expected != self.identity {
            return Err(io::Error::other("content identity changed"));
        }
        for extent in &self.extents {
            if let TreeExtent::Data(content) = extent {
                content.validate()?;
            }
        }
        Ok(())
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        if offset >= self.identity.size {
            return Ok(0);
        }
        let mut start = 0u64;
        for extent in &self.extents {
            let size = match extent {
                TreeExtent::Data(content) => content.len(),
                TreeExtent::Hole(n) | TreeExtent::AllocatedHole(n) => *n,
            };
            if offset < start + size {
                let relative = offset - start;
                let count = usize::try_from((size - relative).min(buffer.len() as u64))
                    .map_err(io::Error::other)?;
                match extent {
                    TreeExtent::Data(content) => {
                        content.read_exact_at(relative, &mut buffer[..count])?
                    }
                    _ => buffer[..count].fill(0),
                }
                return Ok(count);
            }
            start += size;
        }
        Ok(0)
    }
}

/// Owned buffers use the same positional contract as reader-backed payloads.
#[derive(Debug, Clone)]
pub struct BufferContent {
    identity: SourceIdentity,
    bytes: Arc<[u8]>,
}
impl BufferContent {
    pub fn new(object: u128, bytes: impl Into<Arc<[u8]>>) -> Self {
        let bytes = bytes.into();
        Self {
            identity: SourceIdentity {
                object,
                generation: 0,
                size: bytes.len() as u64,
            },
            bytes,
        }
    }
}
impl ContentSource for BufferContent {
    fn identity(&self) -> SourceIdentity {
        self.identity
    }
    fn validate(&self, expected: SourceIdentity) -> io::Result<()> {
        if expected == self.identity {
            Ok(())
        } else {
            Err(io::Error::other("buffer identity changed"))
        }
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        if offset >= self.identity.size {
            return Ok(0);
        }
        let start = usize::try_from(offset).map_err(io::Error::other)?;
        let count = buffer.len().min(self.bytes.len() - start);
        buffer[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(clippy::arc_with_non_send_sync)] // Retain source identity across clones without imposing threading.
    fn repeated_reads_and_extent_holes_are_bounded() {
        let data = DeferredContent::new(Arc::new(BufferContent::new(7, Vec::from(b"abc"))));
        let composite = DeferredContent::new(Arc::new(
            ExtentContent::new(8, vec![TreeExtent::Data(data), TreeExtent::Hole(1 << 40)]).unwrap(),
        ));
        for _ in 0..3 {
            let mut bytes = [9; 5];
            composite.read_exact_at(1, &mut bytes).unwrap();
            assert_eq!(&bytes, b"bc\0\0\0");
        }
        assert!(composite.read_exact_at(u64::MAX, &mut [0]).is_err());
        assert!(composite.read_exact_at(composite.len(), &mut [0]).is_err());
    }
    #[test]
    fn drift_and_short_reads_fail() {
        struct Broken;
        impl ContentSource for Broken {
            fn identity(&self) -> SourceIdentity {
                SourceIdentity {
                    object: 1,
                    generation: 0,
                    size: 10,
                }
            }
            fn validate(&self, _: SourceIdentity) -> io::Result<()> {
                Err(io::Error::other("generation drift"))
            }
            fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
                panic!("drift must be detected before read")
            }
        }
        assert!(
            DeferredContent::new(Arc::new(Broken))
                .read_exact_at(0, &mut [0])
                .is_err()
        );
        let inventory = TreeInventory::default();
        assert!(inventory.validate_budget(0, 0).is_err());
        inventory
            .validate_budget(0, inventory.root_metadata.allocation_bytes())
            .unwrap();
    }
}

#[cfg(feature = "native-writer")]
mod host {
    use super::*;
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
    };
    #[derive(Debug)]
    struct HostContent {
        path: PathBuf,
        identity: SourceIdentity,
    }
    fn identity(path: &Path) -> io::Result<SourceIdentity> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() {
            return Err(io::Error::other("source no longer a regular file"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(SourceIdentity {
                object: (u128::from(metadata.dev()) << 64) | u128::from(metadata.ino()),
                generation: (metadata.mtime() as u64).rotate_left(17)
                    ^ metadata.mtime_nsec() as u64
                    ^ (metadata.ctime() as u64).rotate_left(31)
                    ^ metadata.ctime_nsec() as u64,
                size: metadata.len(),
            })
        }
        #[cfg(not(unix))]
        {
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            path.hash(&mut hash);
            Ok(SourceIdentity {
                object: u128::from(hash.finish()),
                generation: metadata
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(io::Error::other)?
                    .as_nanos() as u64,
                size: metadata.len(),
            })
        }
    }
    pub fn host_file_content(path: impl AsRef<Path>) -> io::Result<DeferredContent> {
        let path = path.as_ref().to_path_buf();
        let identity = identity(&path)?;
        Ok(DeferredContent::new(Arc::new(HostContent {
            path,
            identity,
        })))
    }
    impl ContentSource for HostContent {
        fn identity(&self) -> SourceIdentity {
            self.identity
        }
        fn validate(&self, expected: SourceIdentity) -> io::Result<()> {
            if identity(&self.path)? == expected {
                Ok(())
            } else {
                Err(io::Error::other(
                    "host identity, generation or size changed",
                ))
            }
        }
        fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
            use std::io::{Read, Seek, SeekFrom};
            self.validate(self.identity)?;
            let count = usize::try_from(
                self.identity
                    .size
                    .saturating_sub(offset)
                    .min(buffer.len() as u64),
            )
            .map_err(io::Error::other)?;
            let mut file = fs::File::open(&self.path)?;
            file.seek(SeekFrom::Start(offset))?;
            let result = file.read(&mut buffer[..count])?;
            self.validate(self.identity)?;
            Ok(result)
        }
    }
    /// Host adapter detects path identity, size and timestamp-generation drift.
    /// It does not guarantee an atomic filesystem snapshot; callers requiring
    /// one must supply a snapshot-backed directory or stronger source adapter.
    #[derive(Debug, Clone)]
    pub struct HostTreeSource {
        root: PathBuf,
    }
    impl HostTreeSource {
        pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
            let root = root.as_ref().canonicalize()?;
            if !fs::symlink_metadata(&root)?.is_dir() {
                return Err(io::Error::other("source must be directory"));
            }
            Ok(Self { root })
        }
    }
    impl FileTreeSource for HostTreeSource {
        fn inventory(
            &self,
            maximum_entries: usize,
            maximum_bytes: usize,
        ) -> io::Result<TreeInventory> {
            fn scan(
                root: &Path,
                directory: &Path,
                result: &mut TreeInventory,
                identities: &mut BTreeMap<u128, String>,
                limits: (usize, usize),
                used_bytes: &mut usize,
            ) -> io::Result<()> {
                let mut paths = Vec::new();
                let mut pending_bytes = 0usize;
                for entry in fs::read_dir(directory)? {
                    if paths.len() >= limits.0.saturating_sub(result.entries.len()) {
                        return Err(io::Error::other("inventory entry budget exceeded"));
                    }
                    let path = entry?.path();
                    pending_bytes = pending_bytes
                        .checked_add(std::mem::size_of::<PathBuf>())
                        .and_then(|bytes| bytes.checked_add(path.as_os_str().len()))
                        .ok_or_else(|| io::Error::other("inventory overflow"))?;
                    if used_bytes
                        .checked_add(pending_bytes)
                        .is_none_or(|bytes| bytes > limits.1)
                    {
                        return Err(io::Error::other(
                            "inventory directory scratch budget exceeded",
                        ));
                    }
                    paths.push(path);
                }
                paths.sort();
                if paths.len() > limits.0.saturating_sub(result.entries.len()) {
                    return Err(io::Error::other("inventory entry budget exceeded"));
                }
                for path in paths {
                    let metadata = fs::symlink_metadata(&path)?;
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| {
                            io::Error::other("host native name cannot be represented as UTF-8")
                        })?
                        .to_owned();
                    let relative = path
                        .strip_prefix(root)
                        .map_err(io::Error::other)?
                        .to_str()
                        .ok_or_else(|| io::Error::other("non UTF-8 path"))?
                        .replace('\\', "/");
                    let (object, kind) = if metadata.is_dir() {
                        (0, TreeEntryKind::Directory)
                    } else if metadata.is_symlink() {
                        (
                            0,
                            TreeEntryKind::Symlink(
                                fs::read_link(&path)?
                                    .to_str()
                                    .ok_or_else(|| io::Error::other("non UTF-8 link"))?
                                    .into(),
                            ),
                        )
                    } else if metadata.is_file() {
                        let captured = identity(&path)?;
                        if let Some(target) = identities.get(&captured.object) {
                            (captured.object, TreeEntryKind::HardLink(target.clone()))
                        } else {
                            identities.insert(captured.object, relative.clone());
                            (
                                captured.object,
                                TreeEntryKind::File(vec![TreeExtent::Data(DeferredContent::new(
                                    Arc::new(HostContent {
                                        path: path.clone(),
                                        identity: captured,
                                    }),
                                ))]),
                            )
                        }
                    } else {
                        return Err(io::Error::other("unsupported host object"));
                    };
                    let topology_bytes = match &kind {
                        TreeEntryKind::File(extents) => extents
                            .len()
                            .checked_mul(std::mem::size_of::<TreeExtent>())
                            .ok_or_else(|| io::Error::other("inventory overflow"))?,
                        TreeEntryKind::Symlink(target) | TreeEntryKind::HardLink(target) => {
                            target.len()
                        }
                        TreeEntryKind::Directory => 0,
                    };
                    for amount in [
                        std::mem::size_of::<TreeEntry>(),
                        relative.len(),
                        name.len(),
                        topology_bytes,
                    ] {
                        *used_bytes = used_bytes
                            .checked_add(amount)
                            .ok_or_else(|| io::Error::other("inventory overflow"))?;
                    }
                    if *used_bytes > limits.1 || result.entries.len() >= limits.0 {
                        return Err(io::Error::other("inventory budget exceeded"));
                    }
                    result.entries.push(TreeEntry {
                        path: relative,
                        native_name: name.as_bytes().to_vec(),
                        object,
                        kind,
                        streams: Vec::new(),
                        metadata: crate::preservation::Metadata::default(),
                    });
                    if metadata.is_dir() {
                        scan(root, &path, result, identities, limits, used_bytes)?;
                    }
                }
                Ok(())
            }
            let mut result = TreeInventory::default();
            scan(
                &self.root,
                &self.root,
                &mut result,
                &mut BTreeMap::new(),
                (maximum_entries, maximum_bytes),
                &mut 0,
            )?;
            result.validate_budget(maximum_entries, maximum_bytes)?;
            Ok(result)
        }
    }
}
#[cfg(feature = "native-writer")]
pub use host::{HostTreeSource, host_file_content};
