# mkiso capability inventory

Audit date: 2026-10-06. This inventory separates available library primitives
from measured boot compatibility. The authoritative existing platform evidence
is [ISO-VALIDATION.md](../ISO-VALIDATION.md); this audit does not extend it.

| Capability | Existing primitive and evidence | Planning policy |
| --- | --- | --- |
| ISO9660 data images | `optical-image::write_iso9660_with_options`; levels 1–3, filename policy, Joliet, Rock Ridge metadata, symlinks and multi-extent data. Native writer unit tests and independent xorriso/libarchive extraction are recorded in ISO-VALIDATION.md. | Enable explicitly selected supported options; reject unsupported special files and ambiguous destination names. |
| ISO9660 reading | `iso9660::IsoReader`, bounded reader limits, primary/Joliet/Rock Ridge views and noncontiguous extents. | Reuse the reader; extraction policy remains an orchestration responsibility. |
| UDF reading and writing | `udf::UdfReader`, `UdfImage`, `write_udf_with_cancel`; revisions 1.02, 1.50, 2.00, 2.01, 2.50 and 2.60. Independent reader coverage varies by allocation and partition mode; see crate README and validation document. | Expose only implemented combinations. Revision support alone does not imply every independent reader supports every mode. |
| Windows optical writer | `write_iso_with_cancel` emits UDF 1.02 and ISO9660 boot discovery with an empty ISO root. It requires original `boot/etfsboot.com` and `efi/microsoft/boot/efisys.bin`. Existing fixture evidence includes Windows mounting and BIOS/UEFI/Secure Boot Setup startup. | Preserve this specific profile description; do not describe it as a complete shared-payload ISO9660/UDF bridge. New source media needs independent platform gates. |
| Complete ISO9660/Joliet/UDF bridge | Validation document explicitly identifies this as separate from existing writer paths. | Reject a requested complete bridge until implemented and checked across all views. |
| El Torito catalogs | `BootOptions`, `AdvancedBootOptions`, `BootEntry`, loader patches and platform sections; synthetic structural checks. | Structural support does not prove supplied loaders boot or discover root/payload. |
| Hybrid partition tables | `HybridOptions`, MBR and primary/backup GPT, checksums and embedded EFI addresses; independent sgdisk structural verification. | Do not enable a general Linux USB profile from table generation alone. Caller must supply and validate the complete boot runtime. |
| Rock Ridge relocation, zisofs, special-file authoring | Explicitly outside existing implementation. | Fail with unsupported capability rather than flatten metadata. |
| Cancellation | Optical writers expose checkpoints through layout, emission, hashing and publication. Generic ISO9660 now also exposes actual logical-emission callbacks. Planning APIs report actual hash bytes and discovered entries with buffer/entry-level cancellation checkpoints. | Checkpoints and phase observations are independent; report only the work actually measured. |
| Physical-device writes | `boot-media::device` provides a Linux whole-device writer with serial-bound identity, capacity/alignment checks, exclusive access, mount/holders/swap guards, synchronization and optional full SHA-256 readback after kernel cache invalidation. Tests verify regular-file and missing-authorization rejection; destructive device tests remain unrun. | Require explicit erase and matching expected identity. Only 512-byte-sector, serial-bearing whole disks are currently supported. No automatic GPT relocation, partition expansion, unmounting or trailing-space erase. Cancellation/failure leaves partially written media. |
| USB image creation and Ventoy provisioning | `boot-media::ventoy` provisions regular images through owned Linux loop devices using the official Ventoy 1.1.17 installer and a compiled archive SHA256 pin. MBR/GPT partition bounds, GPT CRCs, exFAT/FAT signatures, runtime identity and copied ISO hashes are checked. | Require the pinned user-supplied archive, root/loop support and external tools. Structural verification does not establish firmware, installer or physical USB compatibility. |
| Optical multiboot | No generic adapter has demonstrated per-entry Linux root discovery or isolated Windows installation payload discovery. | Fail unsupported entry combinations before producing menus. |

The new `boot-media` boundary owns manifests, input fingerprints, operation
reports and profile policy. `mkiso-cli` owns argument parsing and optional terminal
rendering. Neither boundary introduces terminal, Ventoy or device privilege
dependencies into `optical-image`.

## Milestone accounting

| Milestone | Implemented foundation | Remaining completion gate |
| --- | --- | --- |
| M0 | Crate boundaries and this inventory reference existing writer tests and platform evidence. | Continue the audit when enabling any additional profile or adapter. |
| M1 | Strict TOML manifests, bounded deterministic inventories, content and metadata fingerprints, empty-directory accounting, resolved path checks and read-only JSON plans. Saved plans are reconstructed from their manifest before accepting their settings or capability gates. | Exact optical size and patch operations need a public optical layout planner; current plans disclose unknown image size. |
| M2 | Terminal-independent typed events, checked monotonic phase tracking, shared cancellation and optional CLI rendering. Recording tests cover known/unknown/empty totals, overflow, incomplete work and terminal states. | Phase-specific measurements and failure injection must accompany each additional backend. |
| M3–M4 | Existing optical readers/writers provide data ISO creation, inspection, bounded extraction and overlay/repack orchestration. Integration tests cover independent round trips, source mutation, hard-link protection and nonbootable repack overlays. | Repack uses a staged tree; source-backed lazy extents, removal and metadata-edit operations are unavailable. New boot relocation paths need independent reader and firmware evidence; unrecognized boot layouts remain rejected. |
| M5 | Prepared boot fields and boot preparation assets are validated and existing optical primitives are available. | Boot preparation has no firmware evidence. CLI boot profiles remain gated. Complete shared-payload bridge, Linux root discovery and each new Windows payload need separate firmware/installer evidence. |
| M6 | Linux physical whole-device inspection and explicitly authorized writing, synchronization, cancellation and optional full readback; Ventoy partitioned regular-image construction through disposable loop devices. | Destructive physical-device tests have not run. GPT relocation/expansion and provisioning on other host platforms are unavailable. |
| M7 | Original ISO inventories, conservative capacity plans, pinned Ventoy 1.1.17 archive provenance, official Linux loop installation, separate ISO files, documented menu plugins and read-only content verification. The original-five 24 GiB GPT image was built and published successfully in a disposable Debian guest. | All five BIOS cases and Debian/Ubuntu UEFI startup and media/root discovery pass. Windows UEFI normal mode fails with the tested OVMF firmware; its compatibility gate remains open. Completed installations, physical devices, MBR firmware behavior and Secure Boot trust remain separate gates. |
| M8–M9 | Unsupported optical multiboot, persistence, unattended and Secure Boot combinations are disclosed. | Adapter, trust, policy and independent installation gates remain open. |

These foundations do not complete every M0–M9 platform gate in the roadmap.

Windows file-identity checks use `GetFileInformationByHandle` volume/file IDs;
the x86_64 MSVC target compiles successfully. Runtime Windows hard-link and
replacement tests have not run. Physical device operations remain Linux-only.

## Ventoy regular disk images

The enabled route is Linux loop provisioning. The assets directory must contain
`ventoy-1.1.17-linux.tar.gz` from the [official 1.1.17 release](https://github.com/ventoy/Ventoy/releases/tag/v1.1.17),
with SHA256:

```text
7fb4ed08cef6a6b4d39dd19260d8c80291a78dfdf9af7d461571e23cbbc43805
```

The digest is pinned in the backend and checked again on a bounded staged copy
before extraction. `mkiso` does not download Ventoy or operating-system media.
Ventoy's own code is GPL-3.0-or-later; bundled components retain their respective
licenses, as explained by the [upstream licensing page](https://www.ventoy.net/en/doc_license.html).
The Linux release archive does not contain the source license directory. The
versioned [source License notices](https://github.com/ventoy/Ventoy/tree/v1.1.17/License)
provide the individual component notices.

`multiboot build` checks root privileges, `/dev/loop-control`, and executable
`parted`, `mkfs.vfat`, `losetup`, `mount`, `umount`, `tar`, `sh`, `dd` and `sync`
before hashing the ISO inputs. The guest also needs kernel loop and exFAT support.
A disposable Linux guest is an appropriate provisioning environment. The pinned
installer supplies its own additional utilities; a separate host `hexdump` is not
required. Planning remains read-only and does not require root.

```sh
mkiso multiboot plan multiboot.toml --target usb --json
mkiso multiboot build multiboot.toml --target usb --output multiboot.img \
  --size 64GiB --partition-table gpt --data-filesystem exfat \
  --report multiboot-build.json
mkiso multiboot verify multiboot.img --manifest multiboot.toml \
  --report multiboot-verify.json
```

The backend allocates a temporary sibling image, attaches that image to its own
loop device, verifies the backing-file identity, then invokes the [official disk
installer](https://www.ventoy.net/en/doc_start.html) with argument arrays:
`sh Ventoy2Disk.sh -i -S [-g] /dev/loopN`. It does not pass an arbitrary regular
file to the installer and does not provision a physical disk. The installer owns
the boot sectors and partition layout. Its exFAT data partition receives each
original ISO at `/isos/ENTRY_ID.iso`; installers retain separate payloads.
Conservative capacity reservations include EFI space, alignment, metadata and
cluster rounding. MBR images above 2 TiB require GPT. Sizes must be MiB-aligned.

The generated `/ventoy/ventoy.json` sets menu aliases, timeout, search root and
optional default image through documented Ventoy plugins. It explicitly sets
`VTOY_WIN11_BYPASS_CHECK` and `VTOY_WIN11_BYPASS_NRO` to `0`, overriding upstream
bypass defaults. `-S` disables Ventoy Secure Boot runtime support; no firmware
trust store or signing policy is changed. Persistence, unattended installation,
Secure Boot provisioning and optical multiboot remain separate unavailable
profiles. `--reproducible` and fixed timestamps are rejected for this route
because upstream filesystem serials, disk UUIDs and timestamps are not controlled.

Build verifies partition bounds, GPT primary/backup header and table CRCs,
exFAT/FAT signatures, original BIOS runtime bytes with documented UUID/GPT patch
exceptions, and the on-disk EFI runtime version reported by the pinned tooling.
It checks generated menu settings and every copied ISO SHA256, detaches the loop,
synchronizes the file, rechecks inputs and publishes the image. Existing regular
outputs require `--replace`; inputs, assets and the manifest are protected from
aliasing. Reports retain runtime provenance, disk layout, image/ISO hashes and
the successful installer arguments, bounded stdout and stdout digest.

`multiboot verify` binds a supplied manifest, checks disk structures/runtime
identity, mounts only an owned read-only loop device, checks menu and ISO contents,
and rejects an image that changes during verification. This content-verification
route also requires Linux privileges and tooling. Its report has no successful
provisioning transcript because verification did not run an install command.

Subprocesses have a 300-second deadline and an 8 MiB captured-output ceiling.
Cancellation kills the subprocess group and attempts unmount/detach cleanup.
Failed images are retained as `.mkiso-ventoy-*.img` siblings; errors identify the
recovery path and preserve the underlying machine-readable category and exit
status. A failed unmount retains its mount directory; cleanup never recursively
removes an active mounted filesystem. Failure after publication identifies the
published output if synchronization did not complete.

Firmware and installer gates remain unmeasured until exact image-specific
evidence is retained and reviewed. The explicit `multiboot test` command runs
the disposable firmware observer and saves its logs, hashes and screenshots in
`--evidence-dir`; image construction does not start it automatically. Observer
completion or a screenshot alone does not prove menu selection, installer payload
discovery or installation completion. Each gate needs a separate result for the
exact ISO SHA256, backend version/assets, firmware, partition scheme and Secure
Boot state.

## Measured Ventoy provisioning and cleanup

On 2026-10-06, `mkiso-v4` completed the original-five manifest build in a
disposable Debian guest with the pinned Ventoy 1.1.17 runtime. The five original
ISOs total 19,659,231,232 bytes and remain separate in the exFAT partition.
The 24 GiB GPT image was synchronized, unmounted and published at
`/data/cache/mkiso-ventoy-validation/multiboot-gpt.img`, SHA256:

```text
36fa2ec9316d61ef6a1c8f5a203e944bec0529ebd2911609c12b45e04048162f
```

The generated image was subsequently removed at the user's request; its creation
process and historical evidence remain documented in
[the validation record](mkiso-ventoy-validation.md#image-creation-process).

The successful report and installer transcript are retained in
`/data/cache/mkiso-ventoy-validation/build-gpt-v4-report.json`. Its five copied
ISO hashes match the original inventory in
[the firmware/installer validation record](mkiso-ventoy-validation.md).
The report records GPT, exFAT, a 32 MiB FAT16 runtime partition, runtime version
1.1.17 and disabled Secure Boot support. Independent guest `sfdisk --verify`
reported no GPT errors; its JSON ranges match the backend report. `blkid` and
read-only filesystem mounts identified exFAT/FAT16 and all five separate ISO
files. The boot partition contains the upstream EFI loaders. These independent
observations are retained under
`/data/cache/mkiso-ventoy-validation/final-independent-layout/`.

A separate 256 MiB synthetic GPT fixture built and published successfully, then
passed explicit read-only verification with matching image and ISO hashes.
Post-test snapshots showed no remaining loop devices or exFAT mounts. This
regression exercised the fix that closes the generated menu file before
unmounting the data partition. Its image SHA256 is
`1274799a4ef31ce0aa7d3bae9f08a249f317a277786b30d5a731eade8b8c9c5e`;
reports and cleanup snapshots are retained under
`/data/cache/mkiso-ventoy-validation/tiny-cleanup-v4/`.
Read-only inspection of the actual generated menu additionally confirmed that
`timeout_seconds = 0` omits `VTOY_MENU_TIMEOUT`, rather than asking Ventoy to
autoboot immediately; both Windows bypass settings remain `0`.

These results establish the recorded provisioning, content and cleanup gates.
Reviewed final-image cases establish installer startup and selected-media
discovery for all five BIOS entries and Debian/Ubuntu UEFI. Windows UEFI normal
mode fails before Setup with the recorded OVMF builds; WIMBOOT observations
are separately recorded in the validation document. These are exact-image
results and do not establish completed installations, Secure Boot trust, MBR
firmware behavior or physical USB compatibility.

## Provisioning and observer checks

The Ventoy follow-up checks pass: focused all-feature and no-default-feature
Rust suites, focused Clippy with warnings denied, scoped formatting, Windows
MSVC compilation, Python observer protocol tests, and actual KVM host/TCG max
CPU-model smokes. Durable logs are under
`/data/cache/mkiso-ventoy-validation/`. The final explicit read-only verification
also passes and returns the same image SHA256 as the build.

A repeat workspace-wide Clippy/formatting run found unrelated concurrent
`unluac`/`unluac-defender`/`defender-lua` failures. Those changes were preserved;
the earlier workspace pass below is historical evidence, not a claim that the
current unrelated working tree passes those checks.

## Validation limits

The earlier optical/CLI foundation checks on 2026-10-06 passed:

- `cargo test -p boot-media -p mkiso-cli --locked --all-features`:
  9 library unit tests, 9 optical integration tests, 12 planning tests and
  8 CLI integration tests. Optical integration includes independent 7z
  extraction and byte-identical output with recording/no-op observers.
- The corresponding `--no-default-features` run passes with 9 CLI tests,
  including rejection of explicitly requested terminal progress.
- `cargo clippy --all-targets --all-features --locked -- -D warnings` passes
  for the workspace. Scoped formatting checks pass for the three affected crates.
- `cargo check -p boot-media -p mkiso-cli --target x86_64-pc-windows-msvc
  --locked --no-default-features` passes. This is compilation evidence only.
- The complete `optical-image` all-feature test suite passes; ignored large-media
  and platform tests remain separate gates.

Command logs are retained under `/data/cache/mkiso-validation-20261006/`:
`all-features.log`, `no-default-features.log`, `workspace-clippy.log`,
`msvc-check.log` and `optical-image.log`. Feature-on/off CLI test runs use the
same binary output path and must run sequentially to avoid testing a binary
overwritten by the other feature build.

Host tests establish parsing, layout, deterministic output, input rejection and
independent extraction for their exact fixtures. They do not establish universal
BIOS/UEFI behavior, Secure Boot trust, physical USB compatibility, installation
completion or the installed Windows edition/build. Existing Windows Setup
startup evidence belongs to the legacy UDF optical path and its recorded media;
it must not be transferred to a new ISO9660 or multiboot path.

The Ventoy implementation records the exact supplied artifact version, SHA256,
upstream license reference, loop installation route and bounded tool invocation.
Real-image provisioning and firmware evidence are recorded separately in
[the exact-image validation record](mkiso-ventoy-validation.md).
Before enabling a boot profile, retain input/output and loader hashes and separate
firmware, installer-startup and installation-result evidence. Keep artifacts under
`/data/cache`; successful image emission alone is never a compatibility gate.

See [optical-image README](../README.md), its integration
tests (`iso_interop`, `iso9660_extended`, and the UDF interoperability suites),
and [the implementation plan](mkiso-multiboot-plan.md) for interfaces and remaining
milestones.
