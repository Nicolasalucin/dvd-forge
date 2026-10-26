# Ventoy firmware and installer validation

Firmware evidence is specific to an input SHA256, Ventoy asset identity, output
image SHA256, partition scheme, architecture, firmware and Secure Boot state.
A successful image build or visible Ventoy menu does not establish installer
startup, payload discovery, completed installation or physical USB compatibility.

## Exact local input inventory

Read-only SHA256 inventory performed on 2026-10-06. Originals remain at
`/home/rick/fast/ISOs/`. Reproduce an inventory with `python3 scripts/mkiso/installer-inventory.py
--entry ID /absolute/original.iso --output /data/cache/new-inventory.json`
(repeat `--entry` for each source). Existing reports are preserved. The JSON
inventory is retained at
`/data/cache/mkiso-ventoy-validation/input-inventory.json`.

| Entry | Original file | Bytes | SHA256 |
| --- | --- | ---: | --- |
| Debian | debian-13.7.0-amd64-netinst.iso | 792723456 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Ubuntu | ubuntu-24.04.5-live-server-amd64.iso | 4080486400 | 97f3d7ffb032c3eb3b23d2c8be9cc76e60c2c1f2c0146ba5ba9fe01cafae0fd8 |
| Windows 11 | Windows11_24H2.iso | 4957929472 | 78ea5c9b7bc8b3128a567797bd5852ec4f55d9b854c1e654804184793e7b1dde |
| Windows 10 | Windows10_22H2.iso | 4783996928 | af0b22f9643800fa84bdc5556625da816a7a2ac798e9b6923fe98ac4e47e2888 |
| Windows Server | Windows2022_SERVER_EVAL_x64FRE_en-us_202212.iso | 5044094976 | 3e4fa6d8507b554856fc9ca6079cc402df11a8b79344871669f0251535255325 |

Filenames identify source files, rather than independently verified installed
versions. The originals total 19,659,231,232 bytes before layout and free-space
allowances.

Independent `7z l -slt` inspection found distinct Windows payloads:

| Source | Boot image bytes | Installation payload | Payload bytes |
| --- | ---: | --- | ---: |
| Windows 11 | 526410392 | sources/install.esd | 4157761510 |
| Windows 10 | 443748690 | sources/install.esd | 4057847146 |
| Windows Server | 414658108 | sources/install.wim | 4340202461 |

The full listing-derived JSON is at
`/data/cache/mkiso-ventoy-validation/windows-payload-inventory.json`. This is
source structure evidence; it does not prove a booted installer can access it.

## Review gates

Run a separate disposable VM for each entry and firmware combination. Attach
only the assembled multiboot image as boot media and a fresh installation disk.
Use an overlay for writable VM media; preserve the original ISOs and assembled
image. Record menu selection, installer startup, media/root discovery and
installation result separately. A screenshot is an artifact awaiting review,
not an automatic pass.

For Debian and Ubuntu, an installer or live-system screen establishes startup.
Root discovery requires evidence that the selected image's filesystem is
mounted and accessible. Netinst completion also depends on its network/package
sources; record those separately.

For Windows, retain the selected menu entry and WinPE/setup startup screenshots.
Open the setup command prompt and record the discovered media volume, its
`sources/install.wim` or `sources/install.esd` size and `DISM /Get-WimInfo`
output. Compare to the selected source's payload identity. An edition list
alone is insufficient to distinguish shared editions across different media.
Record installed edition, build and version separately after installation.

Exercise BIOS and UEFI independently. Secure Boot remains a separate case,
including the exact enrolled trust state; an unsigned UEFI pass does not imply
Secure Boot success. VM results do not establish physical firmware support.

## Current evidence status

All five input hashes and Windows source payload structures are verified.
Reviewed firmware, installer and media-discovery cases are recorded below.
Completed installation remains untested.

The full original-five build completed successfully using Ventoy 1.1.17, its
official installer on a private loop device in a disposable Debian VM, GPT and
exFAT. Publication and unmount/flush completed. The 24 GiB image was written to
`/data/cache/mkiso-ventoy-validation/multiboot-gpt.img`, SHA256
`36fa2ec9316d61ef6a1c8f5a203e944bec0529ebd2911609c12b45e04048162f`.
The Ventoy archive SHA256 is
`7fb4ed08cef6a6b4d39dd19260d8c80291a78dfdf9af7d461571e23cbbc43805`.
The build report is
`/data/cache/mkiso-ventoy-validation/build-gpt-v4-report.json`. All five source
hashes match the inventory above. Secure Boot and Windows hardware-policy
bypasses were disabled. This records provisioning success; firmware and
installer observations are separate gates.

## Image creation process

The generated multiboot image and the provisioning VM were removed after
validation at the user's request. The original ISOs, manifests, build reports,
hashes and observation evidence remain available. The image hash above describes
the historical build; it is not an available image artifact.

To reproduce the build, compile `mkiso-cli` with locked dependencies and supply
`ventoy-1.1.17-linux.tar.gz` in the manifest's `boot.assets` directory. The
backend checks the archive against the SHA256 above. Use a version-one manifest
with `backend = "ventoy"`, BIOS/UEFI firmware entries and one separate entry for
each original ISO. The recorded manifest is
`/data/cache/mkiso-ventoy-validation/multiboot.toml`; adjust its original ISO and
asset paths to the reproduction environment. Keep source media read-only.

Run provisioning in a disposable Linux environment with root, loop-device
access, the exFAT kernel driver, and `parted`, `mkfs.vfat` (dosfstools),
`losetup`, `mount`, `umount`, `tar`, `sh`, `dd` and `sync` available:

```sh
cargo build -p mkiso-cli --release --locked --features progress
mkiso multiboot plan multiboot.toml --target usb --json
mkiso multiboot build multiboot.toml --target usb --output multiboot.img \
  --size 24GiB --partition-table gpt --data-filesystem exfat \
  --report build-report.json
mkiso multiboot verify multiboot.img --manifest multiboot.toml \
  --report verify-report.json
```

The backend creates a temporary image, attaches an owned loop device, runs the
pinned official installer, and copies each original to `/isos/ENTRY_ID.iso`.
It writes the menu, verifies copied ISO hashes and partition/runtime structures,
closes mounted files, unmounts and detaches the loop, synchronizes the image,
rechecks sources, hashes the output and publishes it. Failed builds retain a
recovery image and report its path. Verification success establishes content
and structure; the firmware/installer results below remain separate gates.

## Scheduled installer observation

`scripts/mkiso/installer-case.py` accepts an explicit image, entry ID, firmware,
firmware files, output directory and action JSON. It runs with KVM, two CPUs,
4 GiB RAM by default, no network, a USB media overlay and a separate blank
64 GiB SATA installation disk. `--swtpm /absolute/swtpm` adds a private TPM 2.0
sidecar. It retains screenshots each second during the first ten seconds, then every
ten seconds, QMP traffic and firmware
hashes. The output directory must be new. Source-image hashes are compared
before and after the case; no observation is marked passed automatically.

An action file is a list of timed key chords and US-layout text, for example:

```json
[
  {"at": 15, "keys": ["ret"]},
  {"at": 25, "keys": ["ret"]},
  {"at": 110, "keys": ["shift", "f10"]},
  {"at": 115, "text": "dir D:\\sources\\install.*\n"}
]
```

Selection timing, menu position and the discovered drive letter must be
established from the screenshots of the exact image. These example actions
are not a compatibility assertion. Never send installation confirmation keys
until the intended disposable destination is independently established.

For Debian's graphical installer, switch to the debug shell with Ctrl+Alt+F2,
press Enter, and record `mount`, `cat /cdrom/.disk/info` and
`ls /cdrom/dists`. Debian documents the graphical-console modifier in its
[installer guide](https://d-i.debian.org/manual/en.amd64/install.en.pdf).
For Ubuntu Subiquity, F2 opens a shell according to the
[official operating guide](https://github.com/canonical/subiquity/blob/main/doc/tutorial/operate-server-installer.rst).
Record `findmnt /cdrom`, `cat /cdrom/.disk/info` and `ls -l /cdrom/casper`.
These commands are proposed probes until the exact selected image is observed.

## Reviewed final-image campaign

The BIOS and UEFI Ventoy menus on the final image display all five entries,
ordered Debian, Server, Ubuntu, Windows 10, Windows 11. The menus wait for
selection. Evidence directories `final-menu-bios-v2` and `final-menu-uefi`
are under `/data/cache/mkiso-ventoy-validation/`.

| Entry | BIOS startup | BIOS media/payload discovery | UEFI startup | UEFI media/payload discovery |
| --- | --- | --- | --- | --- |
| Debian | passed | passed | passed | passed |
| Ubuntu | passed | passed | passed | passed |
| Windows 10 | passed | passed | failed: normal and WIMBOOT | untested |
| Windows 11 | passed with host CPU | passed | failed: normal and WIMBOOT | untested |
| Windows Server | passed | passed | failed: normal and WIMBOOT | untested |

BIOS Debian and Ubuntu evidence is in `final-debian-bios` and
`final-ubuntu-bios`: screenshot 140 shows the mounted selected ISO, its
`.disk/info` identity and installer contents. Debian mounts `/dev/sdb2` at
`/cdrom` and identifies Debian 13.7.0 Trixie NETINST. Ubuntu mounts
`/dev/mapper/ventoy` at `/cdrom`, identifies Ubuntu Server 24.04.5 LTS Noble
and exposes the casper squashfs files. Separate UEFI cases in
`final-debian-uefi` and `final-ubuntu-uefi` show the same selected-media identity
and root/media accessibility in screenshot 140.

BIOS Windows 10 and Server evidence is in `windows-win10-bios` and
`windows-server-bios`. Screenshot 50 shows the respective setup interface;
screenshot 140 shows `D:\sources\install.esd` or `install.wim`, matching the
original sizes listed above. Screenshot 170 shows successful DISM index-1
inspection: Windows 10 Home, version 10.0.19041, service-pack build 2006;
Windows Server 2022 Standard Evaluation, version 10.0.20348, service-pack
build 587. These are payload metadata, rather than completed installations.

The first Windows 11 BIOS case used QEMU's default CPU model and exited after
a Windows boot-logo frame. This failure is retained in `windows-win11-bios`.
A separate `windows-win11-bios-host` case specifies KVM `-cpu host` and reaches
the Windows 11 setup language screen. Screenshot 140 shows the selected
`D:\sources\install.esd`, 4,157,761,510 bytes, matching the original input.
Screenshot 170 shows successful DISM inspection of Windows 11 Home, version
10.0.26100, service-pack build 2033. TPM 2.0, two CPUs and 4 GiB RAM are
present. No Windows hardware-policy bypass was enabled. CPU-model choice is
part of the evidence configuration; the initial failure is not discarded.

Completed installations, Secure Boot and physical firmware remain untested.

All three Windows UEFI normal-mode cases fail before WinPE startup with
`cannot load image` followed by `you need to load the kernel first`. The
separate `windows-{server,win10,win11}-uefi` directories retain screenshot 30
and QMP evidence. Windows WIMBOOT is a separately measured alternative
documented by [Ventoy](https://www.ventoy.net/en/doc_wimboot.html); its outcome
must be stated separately from the failed normal-mode profile.

The pristine plain OVMF variables file and the completed Server normal-mode
variables were inspected with `virt-fw-vars` 25.12. Neither contains enrolled
PK, KEK, db or dbx. Evidence is in `secure-boot-var-inventory.json`,
`pristine-vars.txt` and `server-normal-after-vars.txt`. No trust keys were
changed in response to the boot failure.

All three Windows UEFI WIMBOOT cases also fail the observed startup gate:
the display remains a 1×1 black capture for the bounded 180-second case, and
no installer UI or payload probe is visible. This does not establish the
underlying guest kernel state. Their separate evidence directories are
`windows-server-uefi-wimboot`, `windows-win10-uefi-wimboot` and
`windows-win11-uefi-wimboot`.
Manual review records are retained in `windows-case-reviews.json` and
`linux-case-reviews.json`; raw observer reports intentionally keep observations
unevaluated until review.

A separate Server normal-mode control with plain OVMF 202602 code and pristine
variables also shows the same loader failure, retained in
`windows-server-uefi-202602` screenshot 30. This rules out treating the
202608 firmware version alone as an established cause. All completed cases
retain the unchanged final-image SHA256 in their before/after reports.

A second full read-only inventory after the campaign,
`input-inventory-after.json`, matches every original path, size and SHA256 in
the initial inventory. No original media was modified.

The original Server ISO was then booted directly as read-only optical media,
with the same OVMF 202608, host CPU, two CPUs, 4 GiB RAM and private TPM 2.0.
The scheduled optical control `direct-server-uefi-202608-v2` records a CD/DVD
boot-key prompt in screenshot 4 and the Microsoft Server Operating System
Setup language interface in screenshot 20. This establishes direct original
Server installer startup on that guest configuration. It narrows the failed
multiboot observation to the tested Ventoy UEFI route; it does not identify an
upstream cause or establish other Windows optical gates. The first optical
observation used an early key and is preserved separately rather than treated
as an installer failure.

The direct optical control completed with its original Server ISO SHA256
unchanged (`3e4fa6d8507b554856fc9ca6079cc402df11a8b79344871669f0251535255325`).
Its `report.json` and separate `review.json` retain startup-only evidence;
payload discovery and installation were not evaluated for this control.
