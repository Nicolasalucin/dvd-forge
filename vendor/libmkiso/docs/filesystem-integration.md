# Filesystem integration and release ledger

This development implementation follows `FILESYSTEM_REFACTOR_PLAN.md` without
adding dependencies on partmgr, virtdisk, or an external filesystem core.
The minimum **published** libmkiso version for this complete contract is **0.1.3**.
[libmkiso 0.1.3](https://crates.io/crates/libmkiso/0.1.3) was published and independently
downloaded from crates.io on 2026-10-07. A fresh consumer pinned to `=0.1.3` with
`default-features = false` compiled the parent, native-name, root-ICB and classified
extent APIs without sibling path overrides. Main commit `f41404f` passed
[Linux and Windows CI](https://github.com/CaddyGlow/mkiso-rs/actions/runs/37687602934)
and is the target of `v0.1.3`. Publication is registry evidence, distinct from
firmware, Windows-installation or downstream FAT-builder correctness.

[libmkiso 0.1.4](https://crates.io/crates/libmkiso/0.1.4) was published and downloaded
from crates.io on 2026-10-07. It adds exclusive directory publication on macOS and
Windows, with regression tests protecting existing files and empty directories.
Commit `e9b3870` passed [Linux, Windows and macOS CI](https://github.com/CaddyGlow/mkiso-rs/actions/runs/37689511318).
The [v0.1.4 release workflow](https://github.com/CaddyGlow/mkiso-rs/actions/runs/37689894998)
passed package verification, embedded version checks and release CLI tests on all
three platforms, then published the crate archive, binaries and checksums in the
[GitHub release](https://github.com/CaddyGlow/mkiso-rs/releases/tag/v0.1.4).
The additive topology and classified-extent APIs remain available from 0.1.3.

## Portable retained sources (R1)

All source and reader APIs are available without default features:

```rust,ignore
pub trait ReadAt {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool;
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<usize>;
    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<()>;
}
SliceSource::new(bytes: &[u8]) -> SliceSource<'_>;
BoundedSource::new(source: S, start: u64, length: u64) -> std::io::Result<BoundedSource<S>>;
SourceCursor::new(source: S) -> SourceCursor<S>;
UdfReader::open_source(source: S, limits: UdfLimits) -> UdfResult<UdfReader<'a>>;
UdfReader::open_source_with_limits(source: S, limits: UdfLimits,
    source_limits: udf::SourceLimits) -> UdfResult<UdfReader<'a>>;
UdfReader::read_at(&self, index: usize, offset: u64, output: &mut [u8]) -> UdfResult<usize>;
UdfReader::visit_extents(&self, index: usize, visitor: impl FnMut(u64, Option<u64>, u64) -> UdfResult<()>) -> UdfResult<()>;
```

The source promises fixed length and immutable bytes until its last retained
reader/content handle is dropped. Read-only file access does not establish
snapshot stability. Metadata discovery, partition translation and deferred
payloads read checked ranges; opening never materializes the logical disk.
Offsets are relative to the source, so a bounded region translates its starting
position exactly once. Each ISO `SourceCursor` has its own position. Public ISO
extent offsets stay image-relative and can be used for concurrent positional
payload reads without sharing a seek cursor.

Short positional reads are accepted; exact reads reject premature EOF, excessive
returned lengths, crossing the source end and overflow. File reads clamp to EOF;
unrecorded extents return zeros. Source I/O errors retain their cause; source
adapters can cancel with `Interrupted`. Streams use entry indices and structured
owner information; ICB identity distinguishes UDF hard links.

`udf::SourceLimits` supplies `max_read_bytes: u64`, `max_scratch_bytes: usize`
and a shared `cancelled: Arc<AtomicBool>`. Discovery and deferred reads share
the cumulative requested-read budget. The scratch cap bounds each underlying
source request and extraction buffer; temporary metadata allocations are bounded
separately by `UdfLimits::max_metadata_bytes`. `metadata_bytes()` and
`source_read_bytes()` expose declared charges for measurement. Failures and short
reads still charge the requested chunk. Cancellation also applies to sparse
reads that require no underlying I/O.

`UdfReader::open`, `read_entry`, and `extract` retain their signatures as wrappers.
Because the new reader owns a retained source with a destructor, callers must
drop a borrowed reader before mutating its backing image, including after the
last payload call. This is also required by the immutable-source contract.

## Native topology and classified extents (additive 0.1.3)

These APIs require no default features, and are available on the portable wasm
reader path. Their minimum **crates.io-published** version is **0.1.3**, verified
by a registry-only consumer after publication on 2026-10-07.

```rust,ignore
topology::Parent::{Root, Entry(usize)};
UdfReader::parent(&self, index: usize) -> Option<topology::Parent>;
UdfReader::root_icb(&self) -> udf::IcbIdentity;
IsoReader::topology(&self) -> &iso9660::Topology;
IsoReader::into_topology_parts(self) -> (
    R, iso9660::Index, preservation::Metadata,
    Vec<preservation::Metadata>, iso9660::Topology,
);
iso_tree_source::IsoTreeSource::topology(&self) -> &iso9660::Topology;
udf::UdfExtentKind::{Recorded { source_offset: u64 }, Unallocated, AllocatedUnrecorded};
UdfReader::visit_classified_extents(&self, index: usize,
    visitor: impl FnMut(u64, udf::UdfExtentKind, u64) -> UdfResult<()>) -> UdfResult<()>;
```

`Parent::Entry` addresses a directory occurrence in the same reader's `entries()`;
`Root` is an explicit sentinel outside that index. Parents are recorded while
traversing native directory records, never inferred from display paths. Every
ordinary UDF occurrence has a parent, including separate hard-link names sharing
an ICB. Streams return `None` from `parent`: their existing `StreamInfo::owner`
and `system` fields preserve named, root and system ownership separately. An
invalid entry index also returns `None`. `root_icb()` reports the resolved root
ICB. ISO uses a synthetic, reader-scoped `Parent::Root` identity; no public
record identity or guessed display entry stands in for its root.

`iso9660::Topology` holds parallel `parents: Vec<Parent>` and
`names: Vec<NativeName>`. Semantic leaf identifiers are `NativeName::Primary`
identifier bytes (including versions), `Joliet` UTF-16 units, or `RockRidge` NM
bytes (primary identifier bytes when NM is absent). `Entry::raw_name` always
retains the separately stored directory identifier. Invalid primary bytes remain
explicit; unsupported Joliet surrogates and non-UTF8 Rock Ridge NM are rejected.
Lossy primary display names and stripped versions may collapse display paths,
but distinct native occurrences stay indexed. Indexed extraction remains usable;
the path-based authoring adapter rejects ambiguous display paths before staging.
Existing `Entry` constructors, `Index`, `into_parts`, and `into_inspected_parts`
retain their signatures. Topology and native-name retention are charged to the
existing discovery metadata limits; native directory cycles/aliases and
inconsistent ISO parent/self records are rejected.

The classified visitor does not allocate or read payload. It yields only logical
file bytes, clipping allocation padding and excluding preallocated tails. Partition
translation, embedded payloads and descriptor continuations retain recorded source
offsets and both zero classes. Both zero classes read as zero bytes. The original
`visit_extents` is a wrapper that maps both to `None`, preserving compatibility;
consumers requiring allocation semantics must use the classified visitor. The UDF
deferred inventory maps these classes to `TreeExtent::Hole` and `AllocatedHole`.
Cancellation precedes every callback and discovery resource limits still apply.

A partmgr adapter can map root and directory occurrences directly into its common
TreeSource, use native leaf identifiers for explicit destination conversion, and
inspect shared ICB/Rock Ridge serial identities plus stream and metadata fields
before requesting any FAT destination mutation. Native versions, unsupported
encodings, destination name collisions, hard links, streams and metadata must be
converted explicitly or rejected by that builder's preflight. This repository
provides the codec-owned contract and rejection tests; the actual partmgr FAT
builder integration remains a downstream gate, not a verified libmkiso claim.
The optional public inventory lease extension is not introduced: current private
staging adapters must continue retaining their own reservation, including for
empty/directory-only inventories and staged artifacts.

## Deferred inventories and staging (R2)

Portable `tree_source` exposes `ContentSource`, `SourceIdentity`,
`DeferredContent`, `FileTreeSource`, `TreeInventory`, `TreeEntry`, `TreeEntryKind`,
`TreeExtent`, `TreeStream`, `BufferContent`, and `ExtentContent`. Content identity
contains a source-scoped object number, generation and logical size. Exact
reads validate before and after reading and never allocate a whole payload.
Inventories carry native names separately from destination-compatible paths;
hard-link identities, stream ownership, holes and allocated holes are explicit.
Root and entry metadata accompany the inventory.

```rust,ignore
pub struct SourceIdentity { pub object: u128, pub generation: u64, pub size: u64 }
pub trait ContentSource {
    fn identity(&self) -> SourceIdentity;
    fn validate(&self, expected: SourceIdentity) -> std::io::Result<()>;
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<usize>;
}
pub trait FileTreeSource {
    fn inventory(&self, maximum_entries: usize, maximum_bytes: usize)
        -> std::io::Result<TreeInventory>;
}
DeferredContent::new(source: Arc<dyn ContentSource>) -> DeferredContent;
DeferredContent::read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<()>;
TreeInventory::validate_budget(&self, maximum_entries: usize, maximum_bytes: usize)
    -> std::io::Result<()>;
```

`ContentSource` has no mandatory threading bound: adapters can retain portable
readers that use shared local budgets. Threaded callers must choose sources
whose own types support their concurrency requirements. Entry, stream and
inventory structs are new APIs, not changes to existing `IsoEntry`/`UdfEntry`.

`iso_tree_source::IsoTreeSource::new(IsoReader<SourceCursor<S>>)` and
`udf_tree::UdfTreeSource` retain readers for direct image-to-image content authoring.
`tree_source::HostTreeSource` and `host_file_content` require `native-writer`.
Host adapters detect identity, size and timestamp-generation drift; they do not
claim an atomic snapshot. Use snapshot-backed storage or a stronger adapter
when that assurance is required. Buffers provide a stable owned snapshot.

Native authoring adds these entry points:

```rust,ignore
UdfImage::from_tree_source(source: &impl FileTreeSource, maximum_entries: usize,
                          maximum_bytes: usize) -> anyhow::Result<UdfImage>;
UdfImage::from_tree_source_with_policy(source: &impl FileTreeSource,
    maximum_entries: usize, maximum_bytes: usize, profile: Profile, policy: Policy)
    -> anyhow::Result<(UdfImage, Report)>;
UdfImage::stage_with_cancel(&self, directory: &Path, options: &UdfOptions,
    checkpoint: impl FnMut() -> anyhow::Result<()>)
    -> anyhow::Result<(tempfile::TempPath, String)>;
stage_iso9660_from_tree_source(source: &impl FileTreeSource, directory: &Path,
    options: &IsoOptions, checkpoint: impl FnMut() -> IsoResult<()>)
    -> IsoResult<tempfile::TempPath>;
stage_iso9660_from_tree_source_with_policy(source: &impl FileTreeSource,
    directory: &Path, options: &IsoOptions, policy: Policy,
    checkpoint: impl FnMut() -> IsoResult<()>)
    -> IsoResult<(tempfile::TempPath, Report)>;
```

Staged output is a closed, unpublished temporary host file. Drop removes it;
callers may reopen it, inspect filesystem content, independently audit it, and
then explicitly persist it. Existing publication functions retain their
behavior. No device operation or deployment is authorized by this refactor.
UDF staging includes its ISO boot bridge. ISO rejects named/root/system streams
instead of silently discarding them; its sparse input is transformed into
logical zero bytes. Explicit content-only transformation is not faithful capture.

## Read and write metadata matrices (R3)

`preservation::Field<T>` distinguishes `Absent`, `Uninspected`, `Present(T)`, and
`NotWritable(T)`. `Metadata` describes names, native timestamp bytes and precision,
ownership, permissions, extensions and native opaque fields. Root inspection
is separate from entries. Opaque bytes are never assumed relocatable.

| Profile | Read inspection | Native preservation on write |
| --- | --- | --- |
| ISO primary/Joliet | Stored names and native directory-record timestamps; other uninspected fields remain explicit | Exact native names/timestamps gated |
| ISO Rock Ridge | PX ownership/mode and native TF timestamp representations | Exact fields and hard-link metadata gated |
| UDF 1.02 physical; 1.50 physical/virtual/sparable | Names, FE ownership/permissions and native timestamps, ICB links and sparse extents | Metadata gated; streams unsupported |
| UDF 2.00/2.01 physical/virtual/sparable | FE/EFE fields and associated streams | Content, links, sparse and streams; native metadata gated |
| UDF 2.50/2.60 physical/metadata/metadata-sparable | FE/EFE fields, fragmented/mirrored metadata and associated streams | Content, links, sparse and streams; native metadata gated; plain virtual/sparable authoring unsupported |
| Unknown UDF revisions or pre-2.50 metadata partitions | Unsupported | Unsupported |

`Profile::read_capabilities()` and `write_capabilities()` expose separate
matrices. `preflight(profile, policy, root, entries)` returns a report of every
uninspected/unwritable field. `Policy::ContentOnly` explicitly accepts reported
metadata transformations; `Policy::Faithful` rejects while native metadata
emission remains gated. Unsupported destination profiles always reject.
Writer planning still validates names/collisions, topology, size limits and
stream/revision compatibility before emission.
In staged-writer reports the metadata ordinals enumerate each entry followed
by its streams, then root streams and system streams; root metadata uses `None`.

Advanced preservation remains deliberately unimplemented: exact native-name
transplantation, UDF extended-attribute/implementation-use relocation, full
Rock Ridge metadata authoring, faithful root metadata round trips, Windows
security descriptors/ADS servicing capture, and arbitrary unknown extensions.
No faithful preservation profile is advertised by this release ledger.

## Evidence and separate gates

`tests/positional_source.rs` uses an independently byte-crafted ISO region with
a nonzero prefix, bounds/overflow cases, short reads, retained inventories,
cancellation causes and repeated caller-buffer reads of a 16 TiB virtual source.
`tests/udf_metadata.rs` independently crafts root/file metadata for UDF 2.50 and
2.60, retaining native timestamp bytes and precision. Preservation tests check
uninspected roots, unwritable fields, transformation reporting and unknown
profiles. Deferred-source tests cover holes, repeated reads and identity drift.
Historical interoperability fixtures and validation reports are preserved.

Required checks include fmt, all-feature tests, no-default-feature tests, CLI
without progress tests, clippy with denied warnings, and the wasm32 portable
reader check. Tool-dependent independent readers can skip when absent. Ignored
large-file tests write/extract over 8 GiB and remain separate.

Cancellation/failure tests model cooperative process interruption before
publication. They do not model actual power loss, torn sectors or filesystem
crash durability. Firmware/QEMU/OVMF boot and Windows installation/servicing
correctness need separate disposable environments; Rust tests and successful
image writes do not establish those properties.

## Development validation, 2026-10-07

The required formatting, all-feature, portable, CLI-without-progress, clippy and
WASM checks pass. Tests cover a file-backed reader-to-writer copy with ordinary
and named payloads larger than the inventory budget, root/system streams,
hard-link equivalence, links and holes, plus late source drift before retaining
output. Byte-crafted FE/EFE metadata fixtures cover all six UDF revisions.
The installed p7zip is 17.05; modern UDF independent-reader checks require
7-Zip 20 or later and skip when unavailable. The ignored >8 GiB test and separate
firmware/Windows gates were not run. These results establish development
integration, not publication, power-loss durability or native capture support.
