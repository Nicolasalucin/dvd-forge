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

The rest of the crate is unmodified upstream 0.1.5 (MIT).
