//! Build native-writer fixtures for scripts/test-optical-firmware.py.
use libmkiso::{
    AdvancedBootOptions, BootEmulation, BootEntry, HybridLayout, HybridOptions, IsoLevel,
    IsoOptions, MbrBootPatch, write_iso9660_with_options,
};
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env::args_os().nth(1).ok_or("expected fixture directory")?);
    let source = root.join("source");
    let mbr = fs::read(root.join("isohdpfx.bin"))?;
    for (name, level, layout) in [
        ("level1-mbr", IsoLevel::Level1, HybridLayout::Mbr),
        ("level2-gpt", IsoLevel::Level2, HybridLayout::Gpt),
        ("level3-mbr-gpt", IsoLevel::Level3, HybridLayout::MbrGpt),
    ] {
        let mut bios = BootEntry::bios("isolinux.bin");
        bios.boot_info_table = true;
        write_iso9660_with_options(
            &source,
            &root.join(format!("{name}.iso")),
            &IsoOptions {
                level,
                joliet: true,
                rock_ridge: true,
                advanced_boot: AdvancedBootOptions {
                    entries: vec![bios, BootEntry::efi("efiimg.bin")],
                    ..Default::default()
                },
                hybrid: Some(HybridOptions {
                    layout,
                    mbr_boot_code: mbr.clone(),
                    mbr_patch: MbrBootPatch::Syslinux,
                    efi_partition: Some("efiimg.bin".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )?;
    }
    for (name, emulation, path) in [
        ("bios-no-emulation", BootEmulation::None, "probe.bin"),
        ("bios-floppy", BootEmulation::Floppy1440, "floppy.bin"),
        ("bios-hard-disk", BootEmulation::HardDisk, "harddisk.bin"),
    ] {
        let mut entry = BootEntry::bios(path);
        entry.emulation = emulation;
        if emulation == BootEmulation::HardDisk {
            entry.system_type = 0x01;
        }
        write_iso9660_with_options(
            &source,
            &root.join(format!("{name}.iso")),
            &IsoOptions {
                advanced_boot: AdvancedBootOptions {
                    entries: vec![entry],
                    ..Default::default()
                },
                ..Default::default()
            },
        )?;
    }
    let mut grub = BootEntry::bios("grub.bin");
    grub.boot_info_table = true;
    grub.grub2_boot_info = true;
    write_iso9660_with_options(
        &source,
        &root.join("grub-hybrid.iso"),
        &IsoOptions {
            joliet: true,
            rock_ridge: true,
            advanced_boot: AdvancedBootOptions {
                entries: vec![grub],
                ..Default::default()
            },
            hybrid: Some(HybridOptions {
                mbr_boot_code: fs::read(root.join("grub-mbr.bin"))?,
                mbr_patch: MbrBootPatch::Grub2,
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    Ok(())
}
