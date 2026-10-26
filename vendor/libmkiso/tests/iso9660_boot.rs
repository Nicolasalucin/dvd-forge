#![cfg(feature = "native-writer")]
use libmkiso::{BootImage, BootOptions, IsoOptions, write_iso9660_with_options};
use std::fs;

fn number(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

#[test]
fn bios_only_catalog_preserves_custom_load_segment_and_count() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let payload = vec![0x71; 4096];
    fs::write(source.join("bios.img"), &payload).unwrap();
    let options = IsoOptions {
        boot: BootOptions {
            bios: Some(BootImage {
                path: "bios.img".into(),
                load_segment: 0x1000,
                load_sectors: 8,
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let output = temp.path().join("boot.iso");
    write_iso9660_with_options(&source, &output, &options).unwrap();
    let image = fs::read(output).unwrap();
    let descriptor = image
        .chunks_exact(2048)
        .skip(16)
        .find(|block| block[0] == 0)
        .unwrap();
    let offset = number(&descriptor[71..75]) as usize * 2048;
    let catalog = &image[offset..offset + 2048];
    assert_eq!(&catalog[..2], &[1, 0]);
    assert_eq!(&catalog[32..40], &[0x88, 0, 0, 0x10, 0, 0, 8, 0]);
    assert_eq!(catalog[64], 0);
    let payload_offset = number(&catalog[40..44]) as usize * 2048;
    assert_eq!(
        &image[payload_offset..payload_offset + payload.len()],
        payload
    );
}

#[test]
fn dual_platform_catalog_points_at_original_boot_payloads() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let bios = vec![0x42; 2048];
    let efi = vec![0xef; 4096];
    fs::write(source.join("bios.img"), &bios).unwrap();
    fs::write(source.join("efi.img"), &efi).unwrap();
    let options = IsoOptions {
        boot: BootOptions {
            bios: Some(BootImage::bios("bios.img")),
            efi: Some(BootImage::efi("efi.img")),
        },
        ..Default::default()
    };
    let output = temp.path().join("boot.iso");
    write_iso9660_with_options(&source, &output, &options).unwrap();
    let image = fs::read(output).unwrap();
    let descriptor = image
        .chunks_exact(2048)
        .skip(16)
        .find(|block| block[0] == 0)
        .unwrap();
    assert_eq!(&descriptor[7..30], b"EL TORITO SPECIFICATION");
    let catalog_offset = number(&descriptor[71..75]) as usize * 2048;
    let catalog = &image[catalog_offset..catalog_offset + 2048];
    assert_eq!(&catalog[..4], &[1, 0, 0, 0]);
    assert_eq!(&catalog[30..32], &[0x55, 0xaa]);
    let sum = catalog[..32].chunks_exact(2).fold(0u16, |sum, word| {
        sum.wrapping_add(u16::from_le_bytes([word[0], word[1]]))
    });
    assert_eq!(sum, 0);
    assert_eq!(&catalog[32..40], &[0x88, 0, 0, 0, 0, 0, 4, 0]);
    assert_eq!(&catalog[64..68], &[0x91, 0xef, 1, 0]);
    assert_eq!(&catalog[96..104], &[0x88, 0, 0, 0, 0, 0, 1, 0]);
    let bios_offset = number(&catalog[40..44]) as usize * 2048;
    let efi_offset = number(&catalog[104..108]) as usize * 2048;
    assert_eq!(&image[bios_offset..bios_offset + bios.len()], bios);
    assert_eq!(&image[efi_offset..efi_offset + efi.len()], efi);
}

#[test]
fn efi_only_catalog_uses_efi_validation_platform() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("efi.img"), vec![0; 512]).unwrap();
    let options = IsoOptions {
        boot: BootOptions {
            efi: Some(BootImage::efi("efi.img")),
            ..Default::default()
        },
        ..Default::default()
    };
    let output = temp.path().join("boot.iso");
    write_iso9660_with_options(&source, &output, &options).unwrap();
    let image = fs::read(output).unwrap();
    let descriptor = image
        .chunks_exact(2048)
        .skip(16)
        .find(|block| block[0] == 0)
        .unwrap();
    let offset = number(&descriptor[71..75]) as usize * 2048;
    assert_eq!(&image[offset..offset + 2], &[1, 0xef]);
    assert_eq!(image[offset + 32], 0x88);
    assert_eq!(image[offset + 64], 0);
}

#[test]
fn invalid_boot_images_fail_without_publishing_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let output = temp.path().join("boot.iso");
    for (path, size, sectors) in [
        ("boot.img", 0, 4),
        ("boot.img", 513, 1),
        ("boot.img", 512, 4),
        ("boot.img", 512, 0),
        ("../boot.img", 2048, 4),
        ("missing.img", 2048, 4),
    ] {
        fs::write(source.join("boot.img"), vec![0; size]).unwrap();
        let options = IsoOptions {
            boot: BootOptions {
                bios: Some(BootImage {
                    path: path.into(),
                    load_segment: 0,
                    load_sectors: sectors,
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(
            write_iso9660_with_options(&source, &output, &options).is_err(),
            "{path}: {size}/{sectors}"
        );
        assert!(!output.exists());
    }
}
