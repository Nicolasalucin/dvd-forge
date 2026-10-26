# Modern ISO and multiboot media implementation plan

Date: 2026-10-06
Status: initial implementation present; see [mkiso-capabilities.md](mkiso-capabilities.md)
for implemented commands, restrictions and outstanding platform gates. This plan
continues to describe the full target, including milestones not yet implemented.

## 1. Objective and accepted requirements

Provide a modern `mkiso` CLI and reusable Rust APIs for creating, reading,
editing, inspecting, verifying, and writing optical and bootable disk images.
Support prepared Linux media, Windows installation media, and a menu containing
Debian, Ubuntu, Windows 11, Windows 10, and Windows Server on one USB disk.
Provide a separate optical multiboot target with explicit compatibility gates.

Use `indicatif` behind an optional Cargo feature named `progress`. Display
progress while loading and editing images, writing an ISO or disk image to a
file, writing an image to a device, flushing, and verifying. Progress must reflect
actual work and remain independent of the correctness of the operation.

The examples in this document define the proposed interface. They are not claims
that commands or supported boot combinations already exist.

### Scope boundaries

- Accept prepared media trees, original ISO files, kernels/initramfs images, and
  explicitly supplied bootloader assets.
- Prepare bootloader configuration and media layout; do not silently build a
  Linux distribution, initramfs, or Windows installation payload.
- Keep optical ISO files and partitioned USB disk images distinct output types.
  A `.iso` extension does not establish USB bootability.
- Preserve original input media, Microsoft boot binaries, and source payloads.
- Treat Secure Boot, firmware boot, installer startup, and installation results
  as separately measured capabilities.
- Do not download OS media, bypass Windows hardware checks, rewrite signatures,
  or modify firmware trust configuration automatically.
- Do not make boot, compatibility, or installation claims from successful writes
  alone. Unsupported targets fail during planning rather than generating plausible
  but unverified boot menus.

## 2. Existing repository integration

Reuse `crates/optical-image` for ISO9660/Joliet/UDF reading, layout, and writing.
The crate currently has a `native-writer` feature, ISO and UDF option structures,
and UDF cancellation checkpoints. Audit its actual supported revisions, boot
layouts, and platform evidence before defining profile defaults.

Reuse `archive-fs`/`archive-core` where their filesystem and archive abstractions
fit extraction or overlays. Reuse existing WIM functionality for Windows payload
inspection and splitting only after checking its validation gates. Consult
`ISO-VALIDATION.md` before extending Windows media claims.

Add a dedicated `mkiso` executable, provisionally in `crates/mkiso-cli`, and a
media planning/orchestration library, provisionally `crates/boot-media`. Final
crate names should follow repository conventions after dependency review.
Keep the generic optical writer independent of GRUB, Ventoy, terminal UI, device
privileges, and platform-specific installer policy.

Proposed ownership:

| Component | Responsibility |
| --- | --- |
| `optical-image` | Filesystem structures, readers, deterministic optical layout and emission |
| `boot-media` | Input inventories, typed manifests, profiles, boot adapters, disk layout and verification reports |
| `mkiso-cli` | CLI parsing, JSON/text presentation, `indicatif`, operation orchestration |
| Platform device modules | Device identity, capacity, exclusive access, writes, flush and readback |
| Test harness | Disposable firmware/installer tests and evidence capture |

## 3. CLI contract

```text
mkiso create ROOT --output IMAGE [OPTIONS]
mkiso build MANIFEST [--output IMAGE]
mkiso plan MANIFEST [--json]
mkiso inspect IMAGE [--json]
mkiso verify IMAGE [--manifest MANIFEST] [--json]
mkiso extract IMAGE --output DIRECTORY [OPTIONS]
mkiso repack IMAGE --overlay DIRECTORY --output NEW_IMAGE [OPTIONS]
mkiso boot prepare [OPTIONS]
mkiso multiboot init MANIFEST
mkiso multiboot add MANIFEST ISO --id ID --title TITLE [OPTIONS]
mkiso multiboot plan MANIFEST [--json]
mkiso multiboot build MANIFEST --target usb|optical --output IMAGE [OPTIONS]
mkiso multiboot verify IMAGE [--manifest MANIFEST] [--json]
mkiso multiboot test IMAGE --firmware bios|uefi|bios,uefi [OPTIONS]
mkiso usb build ROOT --profile PROFILE --output IMAGE [OPTIONS]
mkiso disk inspect DEVICE [--json]
mkiso disk write IMAGE --device DEVICE [OPTIONS]
```

Common options:

```text
--progress auto|always|never   Default: auto
--json                       Structured command result on stdout
--report FILE                Save a durable operation/validation report
--replace                    Explicitly replace an existing regular output file
--reproducible               Require all needed deterministic build inputs
--timestamp RFC3339          Fixed filesystem/build timestamp
--jobs N                     Bound supported parallel work; no implicit unbounded workers
```

`--json` enables machine-readable results and suppresses terminal bars in `auto`
mode. Explicit `--progress always` may render bars on stderr. Diagnostics and
progress go to stderr; stdout remains parseable. `--report` must not overwrite
an input or the primary output.

Options are order-independent. Repeatable `--boot KIND=PATH` options identify
distinct boot entries and reject duplicate identifiers. Advanced per-entry
settings belong to the manifest rather than depending on neighboring flags.

Define stable exit statuses for usage, invalid inputs, unsupported capability,
I/O failure, verification failure, resource exhaustion, and cancellation. Add
structured error codes and input/profile/entry context without huge byte dumps.

`plan` performs bounded read-only inspection and presents input hashes, image
size, destination layout, boot entries, bootloader patch operations, filesystem
limits, unsupported combinations, and required assets. It writes no media and
does not boot a VM. A serialized plan includes its schema version and input
fingerprints; `build` rechecks those fingerprints before using a saved plan.

## 4. Single-image creation

### Linux data ISO

```sh
mkiso create ./root --output linux-data.iso --label LINUX_DATA \
  --filesystem iso9660 --rock-ridge --joliet \
  --timestamp 2026-10-06T00:00:00Z
```

Expose filename policy, UID/GID policy, permissions, symlink handling, exclusions,
and timestamp policy explicitly. Never silently flatten Linux filesystem metadata.
Offer strict conflict errors when different source names map to one destination.

### Prepared Linux boot media

```sh
mkiso create ./root --output linux-live.iso --label LINUX_LIVE \
  --profile linux-grub --arch x86_64 \
  --boot bios=./boot/bios.img --boot uefi=./boot/esp.img \
  --boot-assets ./grub-assets --media optical,usb --partition-table gpt \
  --reproducible
```

The profile must verify the relationship between optical boot images, BIOS
system-area code, EFI filesystem image, GRUB configuration, and hybrid disk
layout. A standalone BIOS optical image is not sufficient for BIOS USB boot.
Treat the complete bootloader asset bundle as a versioned, hashed input.
Enable a hybrid profile only after its partition and firmware gates pass.

Optional preparation:

```sh
mkiso boot prepare --loader grub --arch x86_64 \
  --kernel ./vmlinuz --initrd ./initramfs.img \
  --cmdline 'console=tty0 rd.live.image' \
  --assets ./grub-assets --output ./prepared-boot
```

The supplied initramfs remains responsible for locating its root filesystem.
Expose menu generation, relative paths, kernel arguments, multiple initramfs
ordering, architecture, and provided EFI loaders. Do not infer distro-specific
root discovery from kernel filenames alone.

### Prepared Windows installation media

```sh
mkiso create ./windows-media --output windows-install.iso \
  --label WIN_INSTALL --profile windows-install --arch x86_64 \
  --media optical --reproducible
```

Conventional discovery candidates:

| Purpose | Path |
| --- | --- |
| BIOS optical boot image | `boot/etfsboot.com` |
| UEFI optical FAT image | `efi/microsoft/boot/efisys.bin` |
| Windows PE | `sources/boot.wim` |
| Installation image | `sources/install.wim`, `install.esd`, or split `install*.swm` |

Allow explicit overrides and report ambiguity. Never replace a missing boot
asset with a guessed binary. Distinguish x86_64 and arm64 requirements; arm64
must not inherit BIOS support from an x86_64 profile.

The proposed optical default is an ISO9660/UDF bridge with UDF 1.02, subject to
independent reader, Windows mount, large-file, and firmware gates. Files exceeding
4 GiB require verified extent handling and cross-view consistency. Preserve
supplied boot image bytes except for documented, required bootloader patches.
Profiles must distinguish prompt/no-prompt boot images rather than silently
substituting them. Detect payload architecture and report unknown/conflicting
evidence without claiming the installed edition/build is correct.

Windows USB output is a partitioned image:

```sh
mkiso usb build ./windows-media --profile windows-install \
  --output windows-usb.img --filesystem fat32 --split-wim-size 3800MiB
```

Plan WIM splitting before writing, verify every resulting part, and retain the
original WIM. If an oversized payload is ESD or another unsupported representation,
fail with an actionable compatibility error. A FAT32/NTFS split layout or an
additional filesystem driver is a separate profile with its own firmware and
Secure Boot gates. Do not advertise raw optical ISO copying as Windows USB support.

## 5. Manifest schema

Use versioned TOML, strict typed deserialization, and errors for unknown fields.
Paths are relative to the manifest's directory. The example is the intended
schema, not a promise that every combination has an implemented backend.

```toml
version = 1

[image]
source = "./windows-media"
output = "./windows-install.iso"
label = "WIN_INSTALL"
media = ["optical"]

[filesystem]
type = "iso9660+udf"
udf_revision = "1.02"

[boot]
profile = "windows-install"
arch = "x86_64"

[[boot.entries]]
id = "bios"
firmware = "bios"
image = "boot/etfsboot.com"

[[boot.entries]]
id = "uefi"
firmware = "uefi"
image = "efi/microsoft/boot/efisys.bin"

[windows]
boot_wim = "sources/boot.wim"
install_image = "sources/install.wim"

[reproducibility]
timestamp = "2026-10-06T00:00:00Z"
```

Resolve boot-entry image paths against the source media tree; asset bundle paths
and source/output paths resolve against the manifest directory. State this
distinction in validation errors. Reject duplicate entry IDs and unsupported
architecture/firmware combinations. Define the precedence of CLI overrides:
an explicitly supplied CLI value wins, otherwise use the manifest, then verified
profile defaults. Record all resolved values in the build report.

## 6. Multiboot USB

Implement USB first using a pinned Ventoy backend. Ventoy is an external boot
runtime, not an ISO writer feature. Audit source/artifact licensing, asset hashes,
supported firmware, and its documented installation/update procedure before
integration. Invoke external tools with argument arrays, bounded subprocesses,
and explicit reports; do not execute untrusted generated shell fragments.

Keep each original ISO as a separate file in the data partition. Use exFAT as
the proposed initial data filesystem, a separate boot partition, and a selected
MBR/GPT layout. The backend owns the exact required layout; do not synthesize
Ventoy's boot sectors or assume every firmware supports both layouts.

The first implementation must establish how to populate a regular disk image:
evaluate a documented image installer, platform-supported disposable loop device,
or reviewed layout template. Reject unavailable provisioning routes explicitly.
Do not invent a Ventoy command that accepts arbitrary regular image files.
Keep real-device provisioning separate from ordinary file builds.

```toml
version = 1

[menu]
title = "Installation and recovery"
default = "debian"
timeout_seconds = 15

[boot]
backend = "ventoy"
firmware = ["bios", "uefi"]
assets = "./ventoy-assets"

[[entries]]
id = "debian"
title = "Debian"
image = "./isos/debian.iso"

[[entries]]
id = "ubuntu"
title = "Ubuntu"
image = "./isos/ubuntu.iso"

[[entries]]
id = "win11"
title = "Windows 11"
image = "./isos/windows11.iso"

[[entries]]
id = "win10"
title = "Windows 10"
image = "./isos/windows10.iso"

[[entries]]
id = "server"
title = "Windows Server"
image = "./isos/windows-server.iso"
```

```sh
mkiso multiboot plan multiboot.toml
mkiso multiboot build multiboot.toml --target usb \
  --output multiboot.img --size 64GiB --partition-table gpt \
  --data-filesystem exfat
mkiso multiboot verify multiboot.img --manifest multiboot.toml
```

Validate capacity after accounting for partitions, filesystem overhead,
alignment, bootloader assets, and requested free space. Store menu configuration,
input hashes, backend identity, and generated-media hashes. Never combine all
Windows installers into a shared `sources/install.wim` directory.

Entry options may subsequently include unattended configuration, explicitly
selected boot modes, Linux persistence backing files, and deterministic grouping.
Implement these through documented backend adapters; they are not universal ISO
properties. Windows hardware-policy changes remain opt-in and separate from
ordinary media assembly.

### Compatibility reporting

Index evidence by ISO SHA256, backend version/assets, architecture, firmware,
partition scheme, and Secure Boot state. Per-entry statuses are `untested`,
`unsupported`, `failed`, or `passed`, with the stage and evidence reference.
Separate structural validation from firmware startup, installer startup, and
completed installation. Do not print blanket "supported" for a distribution name.

## 7. Optical multiboot

```sh
mkiso multiboot build multiboot.toml --target optical \
  --backend grub --output multiboot.iso
```

Use separate, versioned per-system adapters. Linux loopback boot requires the
kernel/initramfs to find media inside the enclosing ISO. Debian installers and
Ubuntu live images can require different mechanisms. Inspect exact inputs and
test the selected boot path; a GRUB menu entry alone is not a compatibility gate.

Windows requires a verified Windows PE/boot manager/media-discovery strategy.
Merely chainloading a loader from an embedded ISO does not establish installer
payload access. Keep Windows versions isolated and explicitly test that the
selected installer uses its own payload. A future WinPE launcher or another
boot runtime requires independent architecture, memory, licensing, and setup
behavior evidence.

The planner rejects unsupported optical entries. Do not silently substitute USB
output, remove entries, or imply that Ventoy's USB mechanism works on optical
media. Account for physical optical capacity when the user selects a medium;
allow larger ISO files for explicitly selected virtual optical use.

## 8. Loading, editing, extraction, and repacking

Loading means opening a bounded image and indexing filesystem/boot structures;
it does not require reading every payload into memory. Expose lazy payload access
and separate full hashing/verification phases. Progress measures bytes actually
read, never an artificial pass over the whole image just to animate a bar.

Model edits as a source-backed overlay: additions, replacements, removals,
metadata changes, and boot changes. Retain original extents until needed and
build a new output plan without mutating the source image. Reject duplicate
destinations, traversal paths, and ambiguous name mappings. Extraction has an
explicit symlink/special-file policy and bounded counts/depth/output sizes.

```sh
mkiso repack original.iso --overlay ./changes --output updated.iso --boot preserve
```

Require an explicit boot policy when an input is bootable:
`--boot preserve|rebuild|remove`. Preserve logical boot settings and source assets,
but recompute layout-dependent catalogs, LBAs, tables, and required patches when
repacking; byte-copying old metadata after relocating files is not preservation.
Reject an unrecognized boot layout unless it can be preserved correctly or the
user explicitly chooses to remove/rebuild it.

## 9. Progress feature and event API

Use the correctly named crate `indicatif`, optional in the CLI:

```toml
[features]
default = []
progress = ["dep:indicatif"]

[dependencies]
indicatif = { version = "0.18", optional = true }
```

Review and pin the actual dependency through the workspace lockfile at
implementation time. The existing `archive-cli` already provides a useful
`progress` feature precedent. Libraries emit typed events and have no terminal
dependency. Without the feature, all operations and progress callbacks still
work; only the terminal renderer is absent. `auto` and `never` remain accepted;
requesting `always` without the feature returns an actionable usage error.

Proposed library concepts:

```rust
enum ProgressUnit { Bytes, Entries, Operations }

struct ProgressEvent {
    operation_id: u64,
    phase: Phase,
    completed: u64,
    total: Option<u64>,
    unit: ProgressUnit,
    entry_id: Option<String>,
    state: ProgressState,
}

enum ProgressState { Started, Advanced, Finished, Failed, Cancelled }
```

Define separate cancellation checkpoints and observers. An observer receives
events but cannot make a failed write successful. All counters use checked
arithmetic. IDs distinguish parent tasks, phases, and multiboot entries.

| Phase | Measurement and presentation |
| --- | --- |
| Open/index image | Actual metadata bytes or indexed entries; spinner if total is unknown |
| Scan source tree | Entries discovered; spinner until planning establishes a total |
| Hash inputs | Bytes read out of the selected hashing workload |
| Apply overlay | Changed entries/bytes; no misleading source-image byte percentage |
| Plan image | Spinner/current step or known operation count |
| Emit ISO/disk image | Planned logical output bytes successfully emitted |
| Copy image to device | Image bytes successfully accepted by writes, with throughput and ETA |
| Flush/synchronize | Spinner; never mark the operation complete before required synchronization |
| Verify/read back | Bytes actually checked out of the requested verification range |
| Boot tests | Entry/firmware cases completed; describe the current case |

Example:

```text
Writing multiboot.img
████████████░░░░░░░░░░  28.4 / 52.1 GiB  182 MiB/s  ETA 2m 13s
```

Use `MultiProgress` for a parent phase and bounded active entry bars. Default
rendering frequency should be approximately 5–10 Hz; aggregate worker events so
UI locks never occur for every file byte or sector. Event counts must be
monotonic within a phase and finish at the declared total on success.

For random-access writes, seeks do not count as transferred bytes. Distinguish
planned logical emission, physical bytes transferred, and sparse holes. Rewrites
must not produce a percentage exceeding 100%. Avoid a simplistic seekable writer
wrapper as the sole progress source; report from the layout/emission scheduler.
For verification, report the sampled/selected byte range honestly.

Clear or finalize bars on failure/cancellation, preserve a concise diagnostic,
and do not print a success checkmark for an incomplete image. Noninteractive
runs use normal phase diagnostics or an explicitly requested structured event
stream on a separate destination; they never inject progress records into a
single-result stdout JSON document.

## 10. File and device writing

### Regular files

Write through a new temporary sibling, flush and synchronize it, verify the
requested invariants, then publish atomically where supported. Existing outputs
require `--replace`; protect source images, manifests, boot assets, and report
files through canonical path and file-identity checks, including hard links.
Define durability/publication behavior for filesystems lacking atomic replace.
Retain or clean up failed temporary artifacts according to an explicit recovery
policy and identify them in the report.

Recheck source sizes/identity while streaming. Reject changed inputs rather than
publishing an image based on a stale layout. Bound file counts, path depth,
metadata, input bytes, external-tool time/output, and allocation arithmetic.

### Physical devices

```sh
mkiso disk inspect /dev/disk/by-id/usb-DEVICE --json
mkiso disk write multiboot.img --device /dev/disk/by-id/usb-DEVICE \
  --verify full
```

Display stable device identity, serial/model where available, capacity, mounted
volumes, and bytes to overwrite before acquiring destructive authorization.
For unattended operation, require an explicit destructive option and matching
expected identity, for example `--erase --expect-device-id ID`. Revalidate that
identity immediately before writing. Never choose a device automatically.

Require sufficient capacity and supported sector/alignment constraints. Reject
mounted/in-use targets unless the user explicitly requests a supported unmount
flow; distinguish file output from device output rather than relying on filename
extensions. Implement platform-specific exclusive access and privilege errors.
Do not automatically reformat or erase remaining device capacity unless requested.

Cancellation stops at safe checkpoints and reports the device as partially
written. A destructive device write cannot be rolled back by an atomic rename.
Flush/synchronize before success, support explicit readback verification, and
verify only the intended written range without confusing trailing device space
with image content. Boot partitions, GPT backup-header placement on larger
devices, and any expansion step require an explicit plan and separate progress.

## 11. Verification and testing

### Structural tests

- Golden ISO9660/Rock Ridge/Joliet/UDF layouts; independent extraction and hashes.
- Filename collisions, Unicode, permissions, symlinks, timestamps, deep trees,
  empty images, large files, overflow counts, truncated and malformed images.
- Optical boot catalogs, platform IDs, boot image ranges and patch fields.
- GPT/MBR checksums, alignment, protective layout, partition ranges and capacities.
- Windows WIM/SWM discovery and payload separation; EFI filesystem contents.
- Overlay/repack boot relocation, input mutation, output collisions and hard links.
- Deterministic repeated output hashes, including volume IDs, GUID policy,
  filesystem serials, timestamps, ordering and bootloader-generated artifacts.

### Progress and cancellation tests

- Compare output bytes with progress enabled and disabled.
- Record callbacks and assert units, phase ordering, counters and terminal states.
- Cover unknown totals, empty work, sparse output, short writes, rewrites and
  failures before/after flush. Never infer durability from the byte counter.
- Inject write, seek, read, synchronization and cancellation errors.
- Verify stdout JSON remains valid with stderr progress and non-TTY pipelines.
- Test `--progress` behavior both with and without the Cargo feature.
- Use deterministic recording observers for correctness; terminal animation
  snapshots are optional presentation checks rather than the primary gate.

### Boot and installation gates

Use disposable environments and explicitly invoked tests. Building an image
must not boot a VM automatically. Exercise applicable BIOS, UEFI and Secure Boot
configurations separately; retain backend/firmware/image hashes and logs.

For every multiboot entry, test menu selection and installer/live-system startup.
For Windows, verify that each selection discovers its own installation payload;
separately measure installed version/edition/build when that gate is requested.
For Linux, separately measure root discovery and optional persistence.
VM evidence does not establish universal physical-firmware compatibility.
Test representative physical devices/firmware before advertising those profiles.

Run formatting, focused and integration tests, all-target/all-feature Clippy,
and no-default-features checks. Report unavailable external tools and untested
platform combinations explicitly. Preserve unrelated worktree changes.

## 12. Implementation sequence and completion gates

| Milestone | Work | Completion gate |
| --- | --- | --- |
| M0 | Audit existing readers/writers and boot support; finalize crate boundaries/schema | Capability inventory tied to current tests and evidence |
| M1 | Typed manifests, CLI, input inventory and read-only plans | Strict validation, override rules, stable JSON/errors and protected paths |
| M2 | Shared progress/cancellation events and optional `indicatif` renderer | Feature on/off, non-TTY, counter and error-path tests pass |
| M3 | Single data ISO creation and bounded inspection/extraction | Independent reader/hash, reproducibility, resource and large-file gates |
| M4 | Overlay editing and boot-aware repacking | Relocation correctness, lazy loading, source preservation and progress gates |
| M5 | Linux and Windows optical profiles | Applicable BIOS/UEFI, filesystem and installer-startup gates |
| M6 | Partitioned image writer and physical device writer | Layout, identity/capacity, flush/cancellation/readback and platform gates |
| M7 | Ventoy-backed multiboot USB | All five exact ISO inputs verified/tested; backend provision/license gates |
| M8 | Optical multiboot adapters | Each advertised entry proves root/payload discovery and firmware startup |
| M9 | Optional persistence, unattended setup and Secure Boot profiles | Separate policy, trust, compatibility and installation evidence |

M2 precedes writer/editor integration so progress is designed into operations.
M3 and M4 reuse optical primitives rather than implementing another parser.
M5 depends on verified optical layout; M7 depends on the disk writer and a proven
Ventoy provisioning route. M8 remains independent of USB success and may initially
support fewer entries. Unsupported Windows optical combinations remain explicit.

Do not implement a legacy mkisofs option-compatibility layer in the first release.
Document migration recipes for common xorriso/mkisofs uses instead. A future
compatibility frontend may translate arguments into the same typed build plan.

## 13. Evidence and deliverables

Deliver Rust library APIs, the CLI, versioned manifests and JSON reports, profile
documentation, synthetic fixtures, progress/cancellation tests, independent
reader results, and separately recorded firmware/installer evidence.
Keep large media and test artifacts outside the repository, under `/data/cache`.
Record original and output hashes and distinguish verified, inferred, unsupported,
and untested capabilities. Update `ISO-VALIDATION.md` when validation claims change.

Reference interfaces and constraints:

- GNU xorriso: <https://www.gnu.org/software/xorriso/>
- xorriso manual: <https://www.gnu.org/software/xorriso/man_1_xorriso.html>
- GRUB loopback and boot mechanisms: <https://www.gnu.org/software/grub/manual/grub/grub.html>
- Ventoy installation: <https://www.ventoy.net/en/doc_start.html>
- Ventoy disk layout: <https://www.ventoy.net/en/doc_disk_layout.html>

Recheck primary documentation and exact asset versions during implementation.
None of these references establishes compatibility for an untested input image.
