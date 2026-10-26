//! Deferred content-only inventory from an already inspected ISO image.
//!
//! Native metadata preservation needs a separate preflight. ISO has no UDF named,
//! root or system stream namespace, so those inventories are explicitly empty.
use crate::iso9660::{Extent, Index, IsoReader, NativeName, Topology};
use crate::source::{ReadAt, SourceCursor};
use crate::tree_source::{
    ContentSource, DeferredContent, FileTreeSource, SourceIdentity, TreeEntry, TreeEntryKind,
    TreeExtent, TreeInventory,
};
use std::{collections::HashMap, io, sync::Arc};

/// Retains the positional source and public ISO extent index without extracting
/// files or retaining a mutable seek cursor. The input's immutable snapshot
/// contract continues to apply until every returned content handle is dropped.
pub struct IsoTreeSource<S> {
    source: Arc<S>,
    index: Index,
    root_metadata: crate::preservation::Metadata,
    metadata: Vec<crate::preservation::Metadata>,
    topology: Topology,
}
impl<S: ReadAt + 'static> IsoTreeSource<S> {
    pub fn new(reader: IsoReader<SourceCursor<S>>) -> Self {
        let (cursor, index, root_metadata, metadata, topology) = reader.into_topology_parts();
        Self {
            source: Arc::new(cursor.into_inner()),
            index,
            root_metadata,
            metadata,
            topology,
        }
    }
    /// Native namespace relationships retained from the reader.
    pub fn topology(&self) -> &Topology {
        &self.topology
    }
    pub fn index(&self) -> &Index {
        &self.index
    }
}

struct IsoContent<S> {
    source: Arc<S>,
    extents: Vec<Extent>,
    identity: SourceIdentity,
}
impl<S: ReadAt> ContentSource for IsoContent<S> {
    fn identity(&self) -> SourceIdentity {
        self.identity
    }
    fn validate(&self, expected: SourceIdentity) -> io::Result<()> {
        if expected != self.identity {
            return Err(io::Error::other("ISO content identity changed"));
        }
        for extent in &self.extents {
            if extent
                .offset
                .checked_add(extent.size)
                .is_none_or(|end| end > self.source.len())
            {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "ISO source extent exceeds retained image",
                ));
            }
        }
        Ok(())
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        if offset >= self.identity.size || buffer.is_empty() {
            return Ok(0);
        }
        self.validate(self.identity)?;
        let mut logical = 0u64;
        for extent in &self.extents {
            let end = logical
                .checked_add(extent.size)
                .ok_or_else(|| io::Error::other("ISO extent size overflow"))?;
            if offset < end {
                let within = offset - logical;
                let count = (extent.size - within)
                    .min(buffer.len() as u64)
                    .min(self.identity.size - offset) as usize;
                let count_read = self
                    .source
                    .read_at(extent.offset + within, &mut buffer[..count])?;
                if count_read > count {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "ISO source returned excessive read length",
                    ));
                }
                return Ok(count_read);
            }
            logical = end;
        }
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "ISO extents do not cover file",
        ))
    }
}
impl<S: ReadAt + 'static> FileTreeSource for IsoTreeSource<S> {
    fn inventory(&self, maximum_entries: usize, maximum_bytes: usize) -> io::Result<TreeInventory> {
        if self.index.entries.len() > maximum_entries {
            return Err(io::Error::other("inventory entry budget exceeded"));
        }
        // Charge all retained inventory before cloning any variable-size field.
        let mut bytes = self.root_metadata.allocation_bytes();
        if bytes > maximum_bytes {
            return Err(io::Error::other("inventory byte budget exceeded"));
        }
        let longest_path = self
            .index
            .entries
            .iter()
            .map(|entry| entry.name.len())
            .max()
            .unwrap_or(0);
        for (position, entry) in self.index.entries.iter().enumerate() {
            let extent_bytes = self.index.extents[position]
                .len()
                .checked_mul(std::mem::size_of::<Extent>())
                .ok_or_else(|| io::Error::other("inventory overflow"))?;
            for amount in [
                std::mem::size_of::<TreeEntry>(),
                entry.name.len(),
                entry.raw_name.len(),
                entry.name.len() + std::mem::size_of::<&str>(), // display collision scratch
                self.metadata[position].allocation_bytes(),
                extent_bytes,
                std::mem::size_of::<TreeExtent>(),
                entry.link_target.as_ref().map_or(0, String::len),
                longest_path, // conservative potential hard-link target
                std::mem::size_of::<(u32, usize)>(), // serial lookup scratch
            ] {
                bytes = bytes
                    .checked_add(amount)
                    .ok_or_else(|| io::Error::other("inventory overflow"))?;
                if bytes > maximum_bytes {
                    return Err(io::Error::other("inventory byte budget exceeded"));
                }
            }
        }
        // This path-based authoring adapter requires explicit name conversion
        // when distinct native occurrences collapse to one display path.
        let mut paths = std::collections::HashSet::new();
        for entry in &self.index.entries {
            if !paths.insert(entry.name.as_str()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "native ISO names require explicit conversion: duplicate display path",
                ));
            }
        }
        let mut inventory = TreeInventory {
            root_metadata: self.root_metadata.clone(),
            ..TreeInventory::default()
        };
        let mut serials: HashMap<u32, usize> = HashMap::new();
        for (position, entry) in self.index.entries.iter().enumerate() {
            let serial = entry
                .unix
                .as_ref()
                .and_then(|unix| unix.serial)
                .filter(|serial| *serial != 0);
            let object = serial.map_or((1u128 << 127) | position as u128, u128::from);
            let previous = serial.and_then(|serial| serials.get(&serial).copied());
            if let Some(previous) = previous {
                if self.index.entries[previous].size != entry.size
                    || self.index.extents[previous] != self.index.extents[position]
                    || self.index.entries[previous].directory != entry.directory
                    || self.index.entries[previous].link_target != entry.link_target
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "conflicting Rock Ridge hard-link serial",
                    ));
                }
            } else if let Some(serial) = serial {
                serials.insert(serial, position);
            }
            let kind = if let Some(target) = &entry.link_target {
                TreeEntryKind::Symlink(target.clone())
            } else if entry.directory {
                TreeEntryKind::Directory
            } else if let Some(previous) = previous {
                TreeEntryKind::HardLink(self.index.entries[previous].name.clone())
            } else {
                let extents = self
                    .index
                    .extents
                    .get(position)
                    .ok_or_else(|| io::Error::other("missing ISO extent inventory"))?
                    .clone();
                let content = IsoContent {
                    source: self.source.clone(),
                    extents,
                    identity: SourceIdentity {
                        object,
                        generation: 0,
                        size: entry.size,
                    },
                };
                content.validate(content.identity)?;
                TreeEntryKind::File(vec![TreeExtent::Data(DeferredContent::new(Arc::new(
                    content,
                )))])
            };
            inventory.entries.push(TreeEntry {
                path: entry.name.clone(),
                native_name: match &self.topology.names[position] {
                    NativeName::Primary(bytes) | NativeName::RockRidge(bytes) => bytes.clone(),
                    NativeName::Joliet(units) => {
                        units.iter().flat_map(|unit| unit.to_be_bytes()).collect()
                    }
                },
                metadata: self.metadata[position].clone(),
                object,
                kind,
                streams: Vec::new(),
            });
        }
        inventory.validate_budget(maximum_entries, maximum_bytes)?;
        Ok(inventory)
    }
}
