# mkiso-rs

Standalone optical image library. The Rust package is `libmkiso` (formerly `optical-image`).
No sibling checkout is required to build or test this repository. The `mkiso` binary and boot-media orchestration are enabled by the `cli` feature. Archive adapter
and UDF differential fuzz harnesses live in archive-rs.

# libmkiso

Read ISO9660 primary, Joliet and Rock Ridge filesystems through `iso9660::IsoReader`.
The reader validates descriptor byte orders, extent bounds, directory cycles,
and metadata budgets. It streams stored file extents to caller-owned sinks.
Select Joliet with `IsoReader::open_with_options` and `ReadOptions` using
`Namespace::Joliet` or `Namespace::PreferJoliet`. The latter falls back to the
primary namespace when Joliet is absent; `open` retains primary selection.
Level-3 multi-extent files, including noncontiguous sections, are indexed and
extracted in logical order. `Index.extents` lists every section;
`Index.offsets` retains the first section offset for compatibility. The
`archive-core` ISO adapter also extracts all sections. Volume sets remain
unsupported.

Select `Namespace::RockRidge` to require RRIP or `Namespace::PreferRockRidge`
to fall back to Joliet and then primary names. Rock Ridge reads SUSP discovery
and bounded, cycle-checked continuations, alternate names, POSIX mode/uid/gid,
link counts and serial identities, timestamps, device attributes and symbolic
link targets. These appear in `Entry.unix` and `Entry.link_target`; extraction
never follows links or creates special filesystem objects. The archive adapter
continues to select primary ISO names. Relocated Rock Ridge directories and
zisofs compressed payloads are rejected explicitly. Alternate names and link
targets currently require UTF-8.
ISO9660 baseline has no payload checksum; a complete read verifies extent
availability, not payload authenticity.
The parser is available with `default-features = false`. Native creation uses
the `native-writer` feature, enabled by default, and retains the existing
`write_iso` APIs. Browser callers can select the parser without tempfile,
filesystem writer, hashing or random-number dependencies.

Read UDF through `udf::UdfReader::open(&bytes, udf::Limits::default())`.
It indexes caller-owned image bytes and streams file extents with
`extract(index, output)`; `read_entry(index, maximum)` provides bounded buffering.
Supported revisions are 1.02, 1.50, 2.00, 2.01, 2.50 and 2.60. Images
may use multiple physical partitions, virtual/VAT or sparable partition maps,
and metadata partition maps for 2.50/2.60. The reader handles recorded
short/long/extended allocations, embedded data,
Extended File Entries, fragmented metadata, sparse zero regions and bounded
allocation continuations. Metadata mirrors may share or duplicate storage;
descriptor, allocation-chain and directory-checksum failures retry the alternate
mapping before indexing children. This does not repair mixed damage across
both copies or authenticate file payloads.
Extended descriptors support uncompressed recorded data and sparse regions;
implementation-defined encoded or compressed extents are rejected explicitly.

The reader validates tag checksums, descriptor CRC coverage, extent bounds,
partition geometry and directory/allocation cycles. Budgets cover input,
metadata, entries, payload sizes (including streams) and traversal depth. Sparse files stream zeros
in chunks of at most 64 KiB; `read_entry` limits their complete logical size.
File-set and ICB descriptors must fit one block, as must each allocation extent
descriptor. Indirect ICBs and chained file sets are followed with cycle and
metadata budgets. Preallocated tails beyond the information length are checked
without emitting bytes. Primary-anchor failure retries backup anchors and
reserve descriptor sequences.

`entries()` includes ordinary files, directories, symbolic links, named streams
and system streams. Inspect `Entry.kind` before extracting: stream entries have
`Entry.stream` with the owner entry index (or `None` for the root/file set),
stream name and system flag. Extended File Entry object sizes are checked
against the main payload plus indexed stream bytes. Their names belong to their associated stream
namespace and must not be treated as ordinary filesystem paths. Main file names
sharing `Entry.icb` identify hard links. Hard-linked files share one stream
directory, associated with the first indexed owner. Symbolic links expose `link_target`;
extraction returns their UDF encoded pathname bytes and never follows targets.
The `archive-core` adapter retains streams separately from its ordinary entry
namespace. Use its `streams()` iterator and `extract_stream(id, output)` to read
these payloads, `stream_owner(id)` for their ordinary owner ID, and
`link_target(id)` for decoded symbolic link targets. No link or stream
materialization policy is provided by the parser. Directory aliases are rejected
to prevent traversal cycles. Special device files,
multiple volume sequences and revisions beyond 2.60 remain unsupported.
UDF 2.60 reading assumes a logical block image already resolved by the device;
raw physical pseudo-overwrite remapping is not implemented.

The reader is available without `native-writer`; `archive-core` uses this same
parser through its UDF adapter. Native authoring supports the same six revisions.

Create a plain UDF image with `write_udf(source, output)`, or select its revision,
partition map and allocation encoding with `write_udf_with_options` and
`UdfOptions`. Physical partitions, including a split metadata/data layout with
two physical partitions, support every listed revision. Virtual/VAT and
sparable writer profiles support 1.50–2.01; metadata partitions support
2.50/2.60, with either shared or duplicated metadata mirrors and an optional
sparable backing partition. Direct, indirect
and strategy-4096 ICB options are checked against the selected partition profile.
Embedded allocations fall back to external extents when a payload is too large.
Use short or long allocations for broad operating-system compatibility;
extended allocation descriptors have limited support in independent readers.

```rust,no_run
use std::path::Path;
use libmkiso::{AllocationMode, UdfOptions, UdfPartition, UdfRevision,
                    write_udf_with_options};
let options = UdfOptions {
    revision: UdfRevision::V250,
    partition: UdfPartition::Metadata { mirror: true },
    allocation: AllocationMode::Long,
    ..UdfOptions::default()
};
write_udf_with_options(Path::new("media"), Path::new("data.udf"), &options)?;
# Ok::<(), anyhow::Error>(())
```

`UdfImage` builds an explicit tree from source files or byte buffers. It supports
`add_sparse_file` with recorded data and holes, `add_symlink`, `add_hard_link`,
`add_named_stream`, `add_root_stream` and `add_system_stream`. Streams require
2.00 or later; generic stream names beginning with reserved `*UDF` are rejected.
`set_preallocated_blocks` adds checked allocation tails without increasing the
logical payload. Small `UdfOptions.extent_blocks` values cause bounded allocation
continuation chains; `metadata_extent_blocks` selects fragmented metadata
storage. `sparing_packets` records real packet replacements in a sparable map.
`file_set_descriptors` can author chained protected file sets on the physical
write-once profiles; the highest set number is the default. Writer budgets bound
entries, traversal depth, metadata, individual and total logical payloads, and
the resulting image size. Invalid revision/map/allocation combinations fail
before publication. Optional `BootOptions` add the ISO9660 discovery view and
El Torito BIOS/EFI catalog. Outputs are deterministic, refuse existing paths,
and publish after completion with a SHA-256 result; cancellation leaves no
published image. Keep source files unchanged while writing.

These are new logical images. The writer does not append recording sessions,
update existing discs, send drive recording commands or generate physical
pseudo-overwrite remapping metadata. Those device operations are distinct from
authoring a UDF 2.60 logical filesystem image.

```rust,no_run
use libmkiso::iso9660::{IsoReader, Limits};
let file = std::fs::File::open("disc.iso")?;
let mut image = IsoReader::open(file, Limits::default())?;
let mut payload = Vec::new();
image.extract(0, &mut payload)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Write deterministic UDF 1.02 optical media from a directory, with an ISO 9660
boot-discovery view and an El Torito BIOS/EFI catalog. The source must contain
`boot/etfsboot.com` and `efi/microsoft/boot/efisys.bin`.

```rust,no_run
use std::path::Path;
use libmkiso::write_iso_with_hash;

let sha256 = write_iso_with_hash(Path::new("media"), Path::new("windows.iso"))?;
# Ok::<(), anyhow::Error>(())
```

The writer refuses to overwrite an existing output and publishes only after
closing and hashing the image. Source files live in UDF; the ISO 9660 root is
empty. Names are limited to 127 UTF-16 units, and files to 234 short extents
(approximately 234 GiB).

Write a non-bootable ISO9660 level 2 filesystem with `write_iso9660(source,
output)` (requires `native-writer`). This API includes all source files in the
primary filesystem, independently of the existing UDF writer. Creation is
deterministic and refuses to overwrite existing output. Names are converted to
ASCII uppercase; collisions and unsupported names fail rather than being
silently renamed. Directory names permit 1–31 letters, digits or underscores;
file names permit the same characters plus one dot, with at most 30 characters
excluding the dot. File versions are recorded as `;1`. Directory depth is at
most eight including root, and stored paths are limited to 255 bytes.
With default options, files are limited to `u32::MAX` bytes and symlinks and
special files are rejected. Configurable level 3 enables multi-extent files;
Rock Ridge enables symbolic links.
Keep the source tree unchanged during creation; size changes are detected,
but same-size mutations are not. The format follows
[ECMA-119](https://ecma-international.org/publications-and-standards/standards/ecma-119/).

For configurable ISO9660 creation, use `write_iso9660_with_options` with
`IsoOptions`. Set the volume label (1–32 uppercase ASCII letters, digits or
underscores; 16 characters with Joliet), a fixed validated UTC `IsoTimestamp`,
`FilenamePolicy::Strict` or `FilenamePolicy::Mangle`, optional Joliet, and optional
BIOS/EFI `BootImage`s. Defaults retain strict primary names, `ISOIMAGE`,
2000-01-01 UTC and no boot catalog. Mangling produces deterministic primary
aliases; Joliet preserves the original names. Joliet names are limited to 64
UCS-2 characters; non-BMP characters and reserved characters are rejected.

```rust,no_run
use std::path::Path;
use libmkiso::{BootImage, BootOptions, FilenamePolicy, IsoOptions,
                    write_iso9660_with_options};

let options = IsoOptions {
    volume_label: "INSTALL".into(),
    joliet: true,
    filename_policy: FilenamePolicy::Mangle,
    boot: BootOptions {
        bios: Some(BootImage::bios("boot/etfsboot.com")),
        efi: Some(BootImage::efi("efi/microsoft/boot/efisys.bin")),
    },
    ..IsoOptions::default()
};
write_iso9660_with_options(Path::new("media"), Path::new("install.iso"), &options)?;
# Ok::<(), libmkiso::iso9660::Error>(())
```

El Torito supports BIOS-only, EFI-only or dual-platform no-emulation catalogs.
Boot paths must identify regular files relative to the source tree. Images must
be nonempty and 512-byte aligned; load segments and load sector counts are
configurable. BIOS defaults to loading four 512-byte sectors; EFI defaults to
count one, which uses UEFI's device-end rule. Callers supply valid boot loaders
and EFI system-partition images; the crate does not create or validate them.
A boot catalog alone does not establish that the image will boot.

`IsoOptions.level` selects level 1 (8.3 identifiers), level 2 (the default), or
level 3 (multiple recorded sections). `extent_bytes` controls level-3 section
size and must be block-aligned. All namespaces share file payloads.
`joliet_level` selects the supplementary descriptor's conformance level;
`joliet_max_name` defaults to the standard 64 UCS-2 characters and may explicitly
extend it to 103 for compatible readers. `volume_metadata` provides system,
volume-set, publisher, preparer and application identifiers, validated against
both selected namespaces. Writer budgets cover entries, generated metadata and
final image size. Directory depth and primary path-length limits are configurable;
values above 8 and 255 respectively relax baseline ISO restrictions, rather than
performing Rock Ridge directory relocation.

Enable `rock_ridge` to preserve original names independently of primary aliases.
On Unix hosts, source hard links share payload storage and RRIP serial numbers;
symbolic links store their targets without following them, including dangling
links. Permissions and owners default to normalized modes 0644/0755 and uid/gid
zero. Set `unix_metadata.preserve` to retain source modes and owners on Unix,
or configure the normalized fields explicitly. Link counts describe the authored
tree. All timestamps use the fixed `IsoOptions.timestamp`, including RRIP TF
fields; source access/change times are not preserved. Device files, FIFOs and
sockets are not authored.

`advanced_boot.entries` supports up to 31 total El Torito images across x86,
PowerPC, Mac and EFI platform IDs. Entries control bootability, load segment and
count, section selection criteria, no/floppy/hard-disk emulation, and optional
boot-info-table and GRUB2 address patches. Legacy `BootOptions` entries precede
explicit entries. The initial entry cannot contain selection criteria. Floppy
sizes are validated, and hard-disk images require one matching MBR partition.
Patches affect only output bytes; the original loader files remain unchanged.
Catalog developer identification is configurable. Platform entries describe
firmware selection; this library supplies no architecture-specific loaders.

`hybrid` places an MBR or primary/backup GPT in the ISO system area. An embedded
EFI FAT image can serve both El Torito and a disk EFI partition without a second
payload copy. MBR mode exposes the complete ISO and optionally an overlapping
EFI partition; GPT mode exposes the EFI partition with a protective MBR.
`MbrGpt` adds a redundant EFI MBR entry for legacy firmware and is explicitly a
hybrid MBR. GPT arrays and both headers have CRC32 checksums, and the backup
header is the last 512-byte disk sector. The optical volume excludes the appended
backup table. Hybrid images are currently limited to the 32-bit MBR sector range.

Supply compatible MBR bootstrap bytes in `mbr_boot_code`; optional SYSLINUX or
GRUB2 patches point that bootstrap at the selected x86 image. An empty bootstrap
creates partition tables but provides no BIOS USB loader. GUIDs and disk
signatures are deterministic and configurable. EFI images must already contain
a firmware-compatible filesystem and loaders, including the removable-media
fallback path when required. The crate does not validate those contents.

```rust,no_run
use libmkiso::{AdvancedBootOptions, BootEntry, FilenamePolicy,
    HybridLayout, HybridOptions, IsoLevel, IsoOptions, write_iso9660_with_options};
let options = IsoOptions {
    level: IsoLevel::Level3,
    rock_ridge: true,
    joliet: true,
    filename_policy: FilenamePolicy::Mangle,
    advanced_boot: AdvancedBootOptions {
        entries: vec![BootEntry::bios("boot/bios.bin"),
                      BootEntry::efi("boot/efi.img")],
        ..Default::default()
    },
    hybrid: Some(HybridOptions {
        layout: HybridLayout::Gpt,
        efi_partition: Some("boot/efi.img".into()),
        ..Default::default()
    }),
    ..Default::default()
};
write_iso9660_with_options(std::path::Path::new("media"),
                          std::path::Path::new("hybrid.iso"), &options)?;
# Ok::<(), libmkiso::iso9660::Error>(())
```

The extended ISO tests use optional independent `xorriso`, `bsdtar` and `sgdisk`
tools; set `XORRISO`, `BSDTAR` and `SGDISK` to explicit executables. They check
Rock Ridge extraction, independently produced RRIP images, multi-extent data,
boot catalog recognition and GPT integrity. libarchive currently selects primary
names for the tested multi-extent image; xorriso extracts its Rock Ridge name.
xorriso 1.5.6 rejects the 3 KiB symbolic-link target used to exercise chained
continuations; that target has shared-reader regression and fuzz-seed coverage,
with an explicit independent-tool skip for this known path-length limitation.
These are structural and extraction checks, not optical/USB firmware boot tests.
The separate [`firmware harness`](scripts/test-optical-firmware.py) passed
17 executable boot cases with SeaBIOS/OVMF: optical and USB boot across MBR,
GPT and MBR+GPT, BIOS emulation modes, and real SYSLINUX/GRUB loaders with
address patches. See [`ISO-VALIDATION.md`](../windows-uup/ISO-VALIDATION.md) for evidence,
reproduction and limits. UEFI probes run with Secure Boot disabled; no OS
installation or physical hardware compatibility is established by these tests.
Format references: [ECMA-119](https://ecma-international.org/publications-and-standards/standards/ecma-119/),
[libburnia boot layouts](https://raw.githubusercontent.com/Distrotech/xorriso/master/doc/boot_sectors.txt),
and [UEFI GPT](https://uefi.org/specs/UEFI/2.10/05_GUID_Partition_Table_Format.html).

Run `cargo test --locked -p libmkiso` for unit and independent reader tests.
The interoperability tests explicitly skip when neither `7z` nor `7zz` is
available. The large-file gate requires over 8 GiB of writes and extraction:

```sh
cargo test --locked -p libmkiso --test iso_interop -- --ignored
```

Independent byte-crafted UDF tests cover metadata mapping and mirror recovery,
sparse extraction, allocation chains and hostile bounds. Additional producer
checks format empty 1.02–2.01 physical volumes with `mkudffs` using embedded,
short and long allocations, plus 1.50/2.01 VAT open/closed discs and 1.50
sparable CD-RW/DVD-RW profiles with direct/indirect allocation strategies. It skips explicitly if the tool is
unavailable; supply `MKUDFFS` to use an explicit executable:

```sh
MKUDFFS=/path/to/mkudffs cargo test --locked -p libmkiso --test udf_interop --test udf_missing_interop
```

Populated native writer images are independently extracted with current 7-Zip
for all six revisions, including split physical partitions and shared/duplicated
metadata maps. UDFclient checks VAT, sparable and metadata-on-sparable images,
allocation continuations, sparse data and hard-link payloads. Actual packet
remapping is checked by destroying original packet storage before independent
extraction. Configure optional tools explicitly to run these checks:

```sh
SEVEN_ZIP=/path/to/7zz UDFCLIENT=/path/to/udfclient UDFINFO=/path/to/udfinfo cargo test --locked -p libmkiso --test udf_writer_interop
```

Independent readers do not validate every authored feature: 7-Zip rejects
extended descriptors and ignores streams; UDFclient does not expose named
streams and counts preallocated tails as file data. Those features have
byte-crafted and shared-reader regression coverage. `udfinfo` independently
checks recognition sequences and volume profiles for all six revisions.
External producer compatibility for 2.50/2.60 metadata layouts remains an open gate.

These checks do not establish BIOS/EFI bootability or Windows mounting and
installation correctness. Follow the sibling windows-uup repository's `ISO-VALIDATION.md` for those
platform gates. Library callers import this crate directly.

## CLI

```sh
cargo run --features cli --bin mkiso -- --help
cargo build --features cli,progress --release --locked
cargo test --features cli --locked
```

The library-only build leaves CLI dependencies disabled. `progress` enables
`cli` and optional terminal bars. See [CLI usage](docs/cli.md) and
[boot-media APIs](docs/boot-media.md). Historical validation reports retain the
old package names; the current implementation is `libmkiso::boot_media`.

Fuzz harnesses, seed generation and replay commands are documented in
[fuzz/README.md](fuzz/README.md). Run `task fuzz:check` to smoke-test them.

## Public reader API for consumers

Reader APIs are available with `default-features = false`, including on WASM.
The crate root exports `IsoReader`, `IsoReadOptions`, `IsoNamespace`, `IsoLimits`,
`IsoEntry`, `IsoIndex`, `IsoExtent`, `IsoError`, `IsoResult`, and both
`read_iso_index` and `read_iso_index_with_options`. `UnixMetadata` exposes Rock
Ridge attributes. Existing `iso9660` and `rock_ridge` module paths remain valid.

Use `IsoNamespace::PreferRockRidge` for Rock Ridge names with Joliet and primary
fallback, or select a required namespace explicitly. `IsoReader::index()` exposes
all file sections; `into_parts()` transfers the reader and index to an adapter
without a second parse. `read_entry(id, maximum)` buffers a bounded payload;
`extract(id, sink)` streams it.

The UDF exports are `UdfReader`, `UdfLimits`, `UdfEntry`, `UdfEntryKind`,
`UdfStreamInfo`, `UdfIcbIdentity`, `UdfError`, and `UdfResult`. Inspect entry kind,
stream owner, ICB identity, and link target to distinguish ordinary files,
associated streams, hard links, and symbolic links. `read_entry(id, maximum)`
and `extract(id, sink)` provide buffered and streaming output respectively.
`UdfReader::open_source` retains a fixed-length positional `ReadAt` source;
`open(&bytes, limits)` remains the slice adapter. `BoundedSource` translates a
checked image region, and `SourceCursor` supplies independent ISO seek cursors.
See the [filesystem integration ledger](docs/filesystem-integration.md) for
deferred authoring, metadata preservation gates, and publication status.

```rust,no_run
use libmkiso::{IsoNamespace, IsoReadOptions, IsoReader};
let file = std::fs::File::open("image.iso")?;
let reader = IsoReader::open_with_options(file, IsoReadOptions {
    namespace: IsoNamespace::PreferRockRidge,
    ..Default::default()
})?;
for entry in reader.entries() {
    println!("{} {}", entry.size, entry.name);
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Releases

Push a tag matching the Cargo package version, such as `v0.1.0`, to release.
The workflow waits for CI, verifies the tag/version match and crate packaging,
and builds `mkiso` for Linux x86-64, Windows x86-64, and macOS ARM64. Binaries
include the `cli` and `progress` features. It creates a GitHub Release with the
crate package, binary tarballs, the MIT license, and SHA-256 checksums. Tags with
prerelease suffixes create GitHub prereleases.

Publish `libmkiso` separately with `cargo publish --locked` using a crates.io
token authorized for the package before pushing a release tag. Release binaries
embed the tag in `--version`; local builds use the Cargo version unless `MKISO_BUILD_TAG`
is set at compile time.
