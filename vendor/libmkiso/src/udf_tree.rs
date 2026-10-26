//! Content-preserving deferred inventory of a retained UDF reader.
use crate::{
    tree_source::*,
    udf::{EntryKind, UdfReader},
};
use std::{collections::HashMap, io, sync::Arc};

/// Retains the reader and its immutable source for all deferred content handles.
/// Native metadata preservation must be checked separately before authoring.
pub struct UdfTreeSource(pub Arc<UdfReader<'static>>);
struct Payload {
    reader: Arc<UdfReader<'static>>,
    index: usize,
    start: u64,
    identity: SourceIdentity,
}
impl ContentSource for Payload {
    fn identity(&self) -> SourceIdentity {
        self.identity
    }
    fn validate(&self, expected: SourceIdentity) -> io::Result<()> {
        if expected != self.identity {
            return Err(io::Error::other("UDF payload identity changed"));
        }
        Ok(())
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self
            .identity
            .size
            .saturating_sub(offset)
            .min(buffer.len() as u64) as usize;
        if count == 0 {
            return Ok(0);
        }
        self.reader
            .read_at(
                self.index,
                self.start
                    .checked_add(offset)
                    .ok_or_else(|| io::Error::other("offset overflow"))?,
                &mut buffer[..count],
            )
            .map_err(udf_error)
    }
}
fn udf_error(error: crate::udf::Error) -> io::Error {
    match error {
        crate::udf::Error::Io(cause) => cause,
        other => io::Error::other(other),
    }
}
impl UdfTreeSource {
    fn content(&self, index: usize, start: u64, size: u64) -> DeferredContent {
        let icb = self.0.entries()[index].icb;
        #[allow(clippy::arc_with_non_send_sync)]
        DeferredContent::new(Arc::new(Payload {
            reader: self.0.clone(),
            index,
            start,
            identity: SourceIdentity {
                object: (u128::from(icb.partition) << 32) | u128::from(icb.block),
                generation: 0,
                size,
            },
        }))
    }
}
impl FileTreeSource for UdfTreeSource {
    fn inventory(&self, maximum_entries: usize, maximum_bytes: usize) -> io::Result<TreeInventory> {
        let entries = self.0.entries();
        if entries.len() > maximum_entries {
            return Err(io::Error::other("inventory entry budget exceeded"));
        }
        // Charge an upper bound before cloning metadata or allocating handles.
        let mut bytes = self.0.root_metadata().allocation_bytes();
        if bytes > maximum_bytes {
            return Err(io::Error::other("inventory byte budget exceeded"));
        }
        for (index, entry) in entries.iter().enumerate() {
            let metadata = self.0.metadata(index).expect("indexed metadata");
            let mut amount = metadata
                .allocation_bytes()
                .checked_add(entry.name.len())
                .and_then(|n| n.checked_add(entry.raw_name.len()))
                .and_then(|n| {
                    n.checked_add(
                        std::mem::size_of::<TreeEntry>() + std::mem::size_of::<TreeStream>(),
                    )
                })
                .ok_or_else(|| io::Error::other("inventory overflow"))?;
            if let Some(target) = &entry.link_target {
                amount = amount
                    .checked_add(target.len())
                    .ok_or_else(|| io::Error::other("inventory overflow"))?;
            }
            self.0
                .visit_extents(index, |_, recorded, _| {
                    let size = std::mem::size_of::<TreeExtent>()
                        + if recorded.is_some() {
                            std::mem::size_of::<Payload>()
                        } else {
                            0
                        };
                    amount = amount
                        .checked_add(size)
                        .ok_or(crate::udf::Error::ResourceLimit("inventory bytes"))?;
                    Ok(())
                })
                .map_err(udf_error)?;
            bytes = bytes
                .checked_add(amount)
                .ok_or_else(|| io::Error::other("inventory overflow"))?;
            if bytes > maximum_bytes {
                return Err(io::Error::other("inventory byte budget exceeded"));
            }
        }
        let mut inventory = TreeInventory {
            root_metadata: self.0.root_metadata().clone(),
            ..TreeInventory::default()
        };
        let mut indices = HashMap::new();
        let mut objects = HashMap::<u128, String>::new();
        for (index, entry) in entries.iter().enumerate() {
            if entry.stream.is_some() {
                continue;
            }
            let object = (u128::from(entry.icb.partition) << 32) | u128::from(entry.icb.block);
            let kind = match entry.kind {
                EntryKind::Directory => TreeEntryKind::Directory,
                EntryKind::SymbolicLink => TreeEntryKind::Symlink(
                    entry
                        .link_target
                        .clone()
                        .ok_or_else(|| io::Error::other("missing UDF link target"))?,
                ),
                _ => {
                    if let Some(path) = objects.get(&object) {
                        TreeEntryKind::HardLink(path.clone())
                    } else {
                        let mut extents = Vec::new();
                        self.0
                            .visit_classified_extents(index, |logical, kind, length| {
                                extents.push(match kind {
                                    crate::udf::UdfExtentKind::Recorded { .. } => {
                                        TreeExtent::Data(self.content(index, logical, length))
                                    }
                                    crate::udf::UdfExtentKind::Unallocated => {
                                        TreeExtent::Hole(length)
                                    }
                                    crate::udf::UdfExtentKind::AllocatedUnrecorded => {
                                        TreeExtent::AllocatedHole(length)
                                    }
                                });
                                Ok(())
                            })
                            .map_err(udf_error)?;
                        objects.insert(object, entry.name.clone());
                        TreeEntryKind::File(extents)
                    }
                }
            };
            indices.insert(index, inventory.entries.len());
            inventory.entries.push(TreeEntry {
                metadata: self.0.metadata(index).expect("indexed metadata").clone(),
                path: entry.name.clone(),
                native_name: entry.raw_name.clone(),
                object,
                kind,
                streams: Vec::new(),
            });
        }
        for (index, entry) in entries.iter().enumerate() {
            if let Some(info) = &entry.stream {
                let stream = TreeStream {
                    metadata: self.0.metadata(index).expect("indexed metadata").clone(),
                    name: info.name.clone(),
                    native_name: entry.raw_name.clone(),
                    content: self.content(index, 0, entry.size),
                };
                if info.system {
                    inventory.system_streams.push(stream);
                } else if let Some(owner) = info.owner {
                    inventory.entries[*indices
                        .get(&owner)
                        .ok_or_else(|| io::Error::other("missing stream owner"))?]
                    .streams
                    .push(stream);
                } else {
                    inventory.root_streams.push(stream);
                }
            }
        }
        inventory.validate_budget(maximum_entries, maximum_bytes)?;
        Ok(inventory)
    }
}
