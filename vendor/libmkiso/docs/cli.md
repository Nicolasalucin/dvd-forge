# mkiso

Build the CLI with `cargo build -p libmkiso --features cli --locked`. Add `--features progress`
for optional `indicatif` terminal bars. Without that feature `auto` and `never`
are accepted; `always` reports an actionable usage error. JSON results go to
stdout and progress/diagnostics to stderr.

```sh
cargo run -p libmkiso --features cli -- create ./root --output ./data.iso \
  --label LINUX_DATA --filesystem iso9660 --rock-ridge --joliet \
  --timestamp 2026-10-06T00:00:00Z --reproducible
cargo run -p libmkiso --features cli -- plan ./image.toml --json
cargo run -p libmkiso --features cli -- build ./image.toml --report ./build-report.json
cargo run -p libmkiso --features cli -- inspect ./data.iso --json
cargo run -p libmkiso --features cli -- verify ./data.iso --json
cargo run -p libmkiso --features cli -- extract ./data.iso --output ./extracted
cargo run -p libmkiso --features cli -- multiboot init ./multiboot.toml
cargo run -p libmkiso --features cli -- multiboot add ./multiboot.toml ./debian.iso \
  --id debian --title Debian
cargo run -p libmkiso --features cli -- multiboot plan ./multiboot.toml --json
```

A data-image manifest:

```toml
version = 1
[image]
source = "./root"
output = "./data.iso"
label = "LINUX_DATA"
media = ["optical"]
[filesystem]
type = "iso9660"
joliet = true
rock_ridge = true
[reproducibility]
timestamp = "2026-10-06T00:00:00Z"
```

Paths are relative to the manifest directory. Existing regular outputs require
`--replace`. Source aliases and reports overlapping inputs/outputs are rejected.
Extraction requires a new directory and rejects symbolic/special files; it does
not restore POSIX ownership or permissions. Repacking support is deliberately
restricted; see the capability inventory for supported metadata and boot policy.

Linux device writes require `disk inspect DEVICE`, followed by explicit `--erase`
and matching `--expect-device-id ID`; `--verify full` reads back the image range.
Devices without stable serials, mounted/in-use devices, unsupported sector sizes
and regular-file targets are rejected. These APIs have no physical-device test
evidence; no disk is selected automatically.

Exit statuses: 2 usage, 3 invalid input/image, 4 unsupported capability, 5 I/O,
6 verification, 7 resource limit, 130 cancellation. Reports contain schema version
1 and distinguish operation success from boot/installation evidence.

The CLI exposes the planned command families. Linux Ventoy image provisioning
uses a pinned official archive and disposable loop devices; other unavailable boot,
partitioned single-installer USB and optical multiboot routes fail explicitly. See
[the capability inventory](../../docs/mkiso-capabilities.md) and
[implementation plan](../../docs/mkiso-multiboot-plan.md) for remaining gates.
No OS media or bootloader assets are downloaded.

For common mkisofs/xorriso data-image recipes, replace `-o` with `--output`,
`-V` with `--label`, `-J` with `--joliet` and `-R` with `--rock-ridge`.
The CLI has no legacy option-compatibility layer. Boot and hybrid recipes require
a separately validated profile rather than direct option translation.

`build` also accepts a saved JSON plan (`plan --json` or its durable report),
checks schema version and rehashes every input before creating output.
`verify --manifest image.toml` compares full payload hashes and retained names;
this comparison requires Joliet or Rock Ridge. It does not measure boot behavior.
Ctrl-C requests cancellation at library checkpoints; a device can remain partly
written after cancellation.

`boot prepare --loader grub --arch x86_64 --kernel ./vmlinuz
--initrd ./microcode.img --initrd ./initramfs.img --assets ./grub-assets
--output ./prepared` stages the supplied files and writes `boot/grub/grub.cfg`.
The asset bundle must contain `bootx64.efi` (or `bootaa64.efi` for arm64).
Initramfs order is retained. Symbolic links and GRUB scripting/control characters
in `--cmdline` are rejected. This stages configuration and original binaries;
loader embedding/prefix configuration, media root discovery and firmware startup
remain separate validation gates. The result includes asset/payload fingerprints.

Ventoy provisioning accepts `ventoy-1.1.17-linux.tar.gz` in the manifest's asset
directory and checks the compiled official archive SHA-256 before execution.
Run as root inside a disposable Linux VM or another environment with loop-device
and mount access. No physical disk is passed to the installer:

```sh
mkiso multiboot plan multiboot.toml --json
mkiso multiboot build multiboot.toml --target usb --output multiboot.img \
  --size 24GiB --partition-table gpt --data-filesystem exfat --report build.json
mkiso multiboot verify multiboot.img --manifest multiboot.toml --json
mkiso multiboot test multiboot.img --firmware bios,uefi --entry menu \
  --evidence-dir ./firmware-evidence --bios ./bios-256k.bin \
  --ovmf-code ./OVMF_CODE.fd --ovmf-vars ./OVMF_VARS.fd \
  --swtpm /path/to/swtpm --timeout 90 --memory 4096
```

The backend preserves each ISO under `/isos/ID.iso` and writes menu aliases.
Windows 11 hardware and network-requirement bypass controls are explicitly off.
Secure Boot runtime support is disabled in this initial profile; firmware trust
is not changed. Requested reproducible disk builds fail because the upstream
installer has not established deterministic GUIDs, serials and filesystem times.
Failed provisioning retains the temporary image and reports its path.

Firmware tests need Python 3, QEMU, exact supplied BIOS/OVMF files, and KVM access
(or explicitly selected `--accel tcg`). They create private overlays, firmware
variables and optional TPM state. Reports contain observations and evidence;
screen capture alone does not automatically pass firmware or installer gates.
Use [the validation record](../../docs/mkiso-ventoy-validation.md) for the exact
media and measured startup/payload results.

The measured GPT image passes all five BIOS installer/media-discovery cases and
Debian/Ubuntu UEFI cases. Windows UEFI normal and WIMBOOT routes fail startup
with the recorded firmware configuration; direct optical Server startup passes
under the same VM configuration. Do not treat image-build success as Windows
UEFI support. Completed installations, Secure Boot and physical USB tests remain
separate gates.
