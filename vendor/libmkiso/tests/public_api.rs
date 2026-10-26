#![cfg(feature = "native-writer")]
use libmkiso::{
    FilenamePolicy, IsoError, IsoNamespace, IsoOptions, IsoReadOptions, IsoReader, UdfEntryKind,
    UdfLimits, UdfReader, write_iso9660_with_options, write_udf,
};
use std::fs;

#[test]
fn public_iso_api_selects_names_and_preserves_index_after_bounded_reads() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("été.txt"), b"payload").unwrap();
    let image = temp.path().join("image.iso");
    write_iso9660_with_options(
        &source,
        &image,
        &IsoOptions {
            joliet: true,
            filename_policy: FilenamePolicy::Mangle,
            ..Default::default()
        },
    )
    .unwrap();
    let mut reader = IsoReader::open_with_options(
        fs::File::open(&image).unwrap(),
        IsoReadOptions {
            namespace: IsoNamespace::PreferRockRidge,
            ..Default::default()
        },
    )
    .unwrap();
    let id = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "été.txt")
        .unwrap();
    assert!(matches!(
        reader.read_entry(id, 6),
        Err(IsoError::ResourceLimit(_))
    ));
    assert_eq!(reader.read_entry(id, 7).unwrap(), b"payload");
    assert!(matches!(
        reader.read_entry(usize::MAX, 7),
        Err(IsoError::Malformed(_))
    ));
    let extents = reader.index().extents[id].clone();
    let (mut file, index) = reader.into_parts();
    assert_eq!(index.extents[id], extents);
    let primary = libmkiso::read_iso_index(&mut file, libmkiso::IsoLimits::default()).unwrap();
    assert_ne!(primary.entries[id].name, index.entries[id].name);
}

#[test]
fn public_udf_api_lists_and_reads_files() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file.txt"), b"payload").unwrap();
    let image = temp.path().join("image.udf");
    write_udf(&source, &image).unwrap();
    let bytes = fs::read(image).unwrap();
    let reader = UdfReader::open(&bytes, UdfLimits::default()).unwrap();
    let id = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "file.txt")
        .unwrap();
    assert_eq!(reader.entries()[id].kind, UdfEntryKind::File);
    assert_eq!(reader.read_entry(id, 7).unwrap(), b"payload");
}
