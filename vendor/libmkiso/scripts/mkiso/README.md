The firmware observer is an explicitly invoked test tool. It launches a private
QEMU process with networking disabled, a disposable USB-media qcow2 overlay or a
read-only optical image, and a fresh copy of the specified UEFI variables. It
never changes a shared VM or enrolls firmware trust keys.

```sh
python3 scripts/mkiso/test-firmware.py multiboot.img \
  --output /data/cache/mkiso-tests/debian-bios \
  --entry debian --backend ventoy-1.1.17 --firmware bios \
  --bios /path/to/bios-256k.bin \
  --backend-asset /path/to/ventoy-package.tar.gz \
  --key ret --select-after 10 --timeout 90
```

Use `--firmware uefi --ovmf-code CODE.fd --ovmf-vars VARS.fd` for a separate UEFI
case; variables must be the intended pristine template, not a shared guest's
saved state. `--firmware bios,uefi` repeats the same selection independently.
QMP key names such as `down`, `up`, and `ret` are passed in order. `--media optical`
uses a CD-ROM. The default accelerator is TCG; explicitly choose `--accel kvm`
when available.

The tool hashes the input image before and after the run, backend assets,
firmware files, QEMU executable, screenshots, serial log and QMP transcript. A
zero exit code means observation completed. It does **not** certify menu
selection, installer startup, payload/root discovery, installation results,
Secure Boot, or physical firmware compatibility. Those gates remain
`not_evaluated` until independently reviewed using the captured evidence.

SIGINT/SIGTERM stop the private QEMU process and retain evidence. Each case has
its own state directory. Completed cases preserve their overlay and copied
variables for investigation. Tests are bounded to at most one hour per case;
network access inside the guest is disabled.

Run protocol regression tests with:

```sh
python3 -m unittest discover -s scripts/mkiso -p test_firmware_harness.py
```

Windows 11 cases should specify `--memory 4096 --swtpm /path/to/swtpm` with
installer bypasses disabled. Every observer case uses two vCPUs. The optional
TPM is a real TPM 2.0 emulator with private per-case state and control socket;
the observer records its binary hash, command configuration and final state
hashes. It stops the emulator after QEMU. This does not enroll Secure Boot keys
or certify that an installation meets its Windows target specification.

The generic observer uses `-cpu host` under KVM and `-cpu max` under TCG, and
records the selected CPU model. This matters for installers requiring modern
CPU instructions, including Windows 11 24H2.
