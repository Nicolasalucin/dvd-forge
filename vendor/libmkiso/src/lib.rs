//! ISO9660/Joliet/Rock Ridge and UDF readers, with configurable native image writers.

/// Version of this library, as declared in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod iso9660;
pub mod iso_tree_source;
pub mod preservation;
pub mod rock_ridge;
pub mod source;
pub mod topology;
pub mod tree_source;
pub mod udf;
pub use source::{BoundedSource, ReadAt, SliceSource, SourceCursor};

// Format-prefixed names keep both reader APIs usable in the same consumer.
pub use iso9660::{
    Entry as IsoEntry, Error as IsoError, Extent as IsoExtent, Index as IsoIndex, IsoReader,
    Limits as IsoLimits, Namespace as IsoNamespace, ReadOptions as IsoReadOptions,
    Result as IsoResult, read_index as read_iso_index,
    read_index_with_options as read_iso_index_with_options,
};
pub use rock_ridge::UnixMetadata;
pub use udf::{
    Entry as UdfEntry, EntryKind as UdfEntryKind, Error as UdfError, IcbIdentity as UdfIcbIdentity,
    Limits as UdfLimits, Result as UdfResult, StreamInfo as UdfStreamInfo, UdfExtentKind,
    UdfReader,
};

#[cfg(feature = "native-writer")]
mod el_torito;
#[cfg(feature = "native-writer")]
mod hybrid;
#[cfg(feature = "native-writer")]
mod iso9660_writer;
#[cfg(feature = "native-writer")]
mod iso_options;
#[cfg(feature = "native-writer")]
pub use el_torito::{AdvancedBootOptions, BootEmulation, BootEntry, BootImage, BootOptions};
#[cfg(feature = "native-writer")]
pub use hybrid::{HybridLayout, HybridOptions, MbrBootPatch};
#[cfg(feature = "native-writer")]
pub use iso_options::{
    FilenamePolicy, IsoLevel, IsoOptions, IsoTimestamp, UnixMetadataOptions, VolumeMetadata,
};
#[cfg(feature = "native-writer")]
pub use iso9660_writer::{
    stage_iso9660_from_tree_source, stage_iso9660_from_tree_source_with_policy, write_iso9660,
    write_iso9660_with_options, write_iso9660_with_options_and_cancel,
    write_iso9660_with_options_and_progress,
};

#[cfg(feature = "native-writer")]
mod udf_boot;
#[cfg(feature = "native-writer")]
pub mod udf_writer;
#[cfg(feature = "native-writer")]
pub use udf_writer::{
    AllocationMode, IcbStrategy, UdfFileExtent, UdfImage, UdfOptions, UdfPartition, UdfRevision,
    write_udf, write_udf_with_cancel, write_udf_with_options,
};
#[cfg(feature = "native-writer")]
mod writer;
#[cfg(feature = "native-writer")]
pub use writer::{write_iso, write_iso_with_cancel, write_iso_with_hash};

/// Media planning and orchestration used by the optional mkiso CLI.
#[cfg(feature = "cli")]
pub mod boot_media;

pub mod udf_tree;
