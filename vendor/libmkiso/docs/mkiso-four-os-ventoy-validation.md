# Four-system Ventoy boot test — 2026-10-06

Created a 20 GiB GPT/exFAT USB disk image using the pinned official Ventoy
1.1.17 Linux archive. Each original ISO remains a separate, unmodified file.
The main Ventoy menu waits for manual selection: no timeout or default-image
control is emitted. Windows hardware and online-account bypasses are disabled.

Artifact directory: `/data/cache/mkiso-ventoy-validation`.

- Disk image: `four-os-ventoy.img`.
- Host manifest: `four-os.toml`; provisioning manifest: `four-os-guest.toml`.
- Structural/content verification: `four-os-manual-verified.json` (status `ok`).
- Boot results: `four-os-boot-results.json` and the referenced PNG screenshots.
- Interactive VM launcher: `boot-four-os.sh`; VNC display during this test: `:7`.

Image SHA256:
`22be8916306a58712eeebef643fd79c069d0cd9d7b03eecdafd0f62b26b5bbbe`.

## Observed installer startup

| Menu entry | Source ISO | Tested firmware | Result |
| --- | --- | --- | --- |
| Debian 13.7 | `debian-13.7.0-amd64-netinst.iso` | OVMF 202608 UEFI | Debian installer language selection |
| Ubuntu 26.04.1 Server | `ubuntu-26.04.1-live-server-amd64.iso` | OVMF 202602 UEFI | Subiquity language selection |
| Windows 10 22H2 | `Windows10_22H2.iso` | SeaBIOS | Windows Setup language selection |
| Windows 11 24H2 | `Windows11_24H2.iso` | SeaBIOS | Windows 11 Setup language selection |

Windows normal-mode UEFI handoff failed with `cannot load image` under OVMF
202608. A writable USB snapshot did not resolve that result. WIMBOOT UEFI
startup was not established. These results do not establish Windows UEFI
compatibility or Windows 11 installation eligibility under legacy BIOS.
Secure Boot was disabled. No installation was completed, installer payload
selection was not tested, and no physical USB device was written.

## Build recovery and fixes

The initial build copied and checked the ISO payloads but failed to unmount
because its menu configuration file was still open. The retained recovery
image was recovered, its menu changed to require manual selection, then
independently verified with `mkiso multiboot verify --manifest`.

The backend now closes the configuration file before unmounting. It also omits
`VTOY_MENU_TIMEOUT` when `timeout_seconds = 0`: Ventoy's explicit zero boots
immediately, while omission waits for input. Regression coverage checks both
zero and positive timeout policies.

Validation: all 43 boot-media tests passed; Clippy for boot-media and mkiso-cli
passed with all targets/features and warnings denied. Full workspace Clippy
was blocked by an unrelated `needless_update` in
`crates/defender-lua/examples/coverage.rs:114`.
