# DVD Forge local patch

`libmkiso` 0.1.5 writes the UDF 2.50 metadata file entry as a short
ECMA-167 File Entry (descriptor tag 261, short allocation descriptors).
UDF 2.50 requires an Extended File Entry (tag 266) with long allocation
descriptors for the Metadata File and Metadata Mirror File. libudfread and
hardware Blu-ray players therefore reject the metadata partition.

`src/udf_writer_volume.rs::metadata_entry` is patched to emit the extended
entry layout:

- ICB allocation-descriptor type 1 (long)
- Extended File Entry header (216 bytes)
- L_EA at offset 208, L_AD at offset 212
- 16-byte long allocation descriptors pointing at partition 0
- descriptor tag 266

The reader (`src/udf_partition.rs`) is likewise relaxed to accept long
allocation descriptors for tag 266 metadata entries while retaining support
for older short-entry images.

`src/udf_writer_volume.rs::write_metadata_entry` is patched to keep the same
extended encoding when the metadata file needs an allocation-extent chain:
the first entry and every AED now use 16-byte long descriptors (113 + 125 per
block) and tag 266/258 instead of the old 8-byte short descriptors.

`src/udf_writer.rs` also adds `UdfOptions::compute_digest` (enabled by
default) so callers that do not need the SHA-256 of the finished image can
skip the extra read pass, and uses 4 MiB payload/hash buffers.

Additional local hardening on top of upstream 0.1.5:

- Publication uses `TempPath::persist_noclobber`, so an existing output is
  never overwritten even on exFAT/FAT32/network volumes where a bare hard
  link is unavailable.
- Non-VAT UDF images end on a 32-sector (64 KiB) boundary, keeping the
  trailing AVDPs inside the last BD ECC cluster / two DVD ECC blocks.
- `CheckedWriter` cancellation uses a non-retryable error kind so
  `std::io::copy` does not spin on `ErrorKind::Interrupted`.
- `BLKGETSIZE64` is derived from `size_of::<size_t>()`, fixing optical device
  sizing on i686/armv7.
- UDF limits accept single BD M2TS files over 8 GiB and BD-50/BD-XL payloads
  over 32 GiB; `read_entry` reserves exactly once and rejects sizes that do
  not fit the address space.
- The ISO and UDF writers use 1 MiB payload/hash buffers to reduce deferred
  source re-opens and syscalls.

The rest of the crate is upstream 0.1.5 (MIT) plus these local patches.
