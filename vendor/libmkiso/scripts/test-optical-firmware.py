#!/usr/bin/env python3
"""Boot native libmkiso fixtures in isolated SeaBIOS/OVMF QEMU guests.

Requires cargo, QEMU, NASM, clang/lld-link, mtools and SYSLINUX assets.
Unsigned probes test UEFI with Secure Boot disabled; no guest disks or network.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


def run(args):
    subprocess.run([str(arg) for arg in args], check=True)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--syslinux", required=True, type=Path)
    parser.add_argument("--firmware", required=True, type=Path)
    parser.add_argument("--bios", type=Path, help="SeaBIOS binary (default: QEMU share/qemu/bios-256k.bin)")
    parser.add_argument("--grub", required=True, type=Path, help="GRUB i386-pc module directory")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout", type=int, default=45)
    args = parser.parse_args()
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    source = root / "source"
    source.mkdir()
    for name in ["isolinux.bin", "ldlinux.c32"]:
        shutil.copyfile(args.syslinux / name, source / name)
    shutil.copyfile(args.syslinux / "isohdpfx.bin", root / "isohdpfx.bin")
    (source / "isolinux.cfg").write_text(
        "SERIAL 0 115200\nDEFAULT probe\nPROMPT 0\nLABEL probe\n"
        " KERNEL linux.bin\n"
    )
    # A 16-bit boot sector works at 0000:7c00 for all three BIOS emulations.
    assembly = root / "probe.asm"
    assembly.write_text("""bits 16
org 0x7c00
cli
cld
xor ax, ax
mov ds, ax
mov ss, ax
mov sp, 0x7c00
mov dx, 0x3fb
mov al, 0x80
out dx, al
mov dx, 0x3f8
mov al, 1
out dx, al
inc dx
xor al, al
out dx, al
mov dx, 0x3fb
mov al, 3
out dx, al
mov si, message
next:
lodsb
test al, al
jz done
mov bl, al
wait_serial:
mov dx, 0x3fd
in al, dx
test al, 0x20
jz wait_serial
mov al, bl
mov dx, 0x3f8
out dx, al
jmp next
done:
mov dx, 0xf4
mov al, 0x10
out dx, al
hlt
jmp done
message: db 'FIRMWARE_BOOT_PASS_BIOS', 13, 10, 0
times 510-($-$$) db 0
dw 0xaa55
""")
    sector = root / "sector.bin"
    run(["nasm", "-f", "bin", assembly, "-o", sector])
    payload = sector.read_bytes()
    (source / "sector.bin").write_bytes(payload)
    (source / "probe.bin").write_bytes(payload + bytes(1536))
    (source / "floppy.bin").write_bytes(payload + bytes(1440 * 1024 - 512))
    disk = bytearray(payload + bytes(4 * 1024 * 1024 - 512))
    disk[446:462] = bytes([0x80, 0, 2, 0, 1, 0xfe, 0xff, 0xff]) + (1).to_bytes(4, "little") + (8191).to_bytes(4, "little")
    (source / "harddisk.bin").write_bytes(disk)
    # Linux boot-protocol setup stub: SYSLINUX loads it, then our real-mode
    # payload reports success. It is deliberately not an operating-system test.
    setup = assembly.read_text().split("org 0x7c00\n", 1)[1]
    setup = setup.replace("xor ax, ax\nmov ds, ax\nmov ss, ax\nmov sp, 0x7c00",
                          "push cs\npop ds\npush cs\npop ss\nmov sp, 0xfff0")
    setup = setup.replace("mov si, message", "call find_message\nfind_message:\npop si\nadd si, message-find_message")
    setup = setup.replace("times 510-($-$$) db 0\ndw 0xaa55\n",
                          "times 2560-($-$$) db 0\ntimes 512 db 0\n")
    linux_assembly = root / "linux.asm"
    linux_assembly.write_text("bits 16\norg 0\ntimes 0x1f1 db 0\ndb 4\ndw 0\ndd 32\n"
                              "times 510-($-$$) db 0\ndw 0xaa55\njmp short setup_start\n"
                              "db 'HdrS'\ndw 0x0200\ntimes 0x268-($-$$) db 0\nsetup_start:\n" + setup)
    run(["nasm", "-f", "bin", linux_assembly, "-o", source / "linux.bin"])
    # Freestanding PE/COFF EFI application; entry follows the Microsoft x64 ABI.
    c_source = root / "probe.c"
    c_source.write_text("""
static void out(unsigned short p, unsigned char v) {
    __asm__ volatile("outb %0,%1" : : "a"(v), "Nd"(p));
}
static unsigned char in(unsigned short p) {
    unsigned char v; __asm__ volatile("inb %1,%0" : "=a"(v) : "Nd"(p)); return v;
}
unsigned long long efi_main(void *image, void *system) {
    (void)image; (void)system;
    out(0x3fb, 0x80); out(0x3f8, 1); out(0x3f9, 0); out(0x3fb, 3);
    const char *s = "FIRMWARE_BOOT_PASS_UEFI\\r\\n";
    while (*s) { while (!(in(0x3fd) & 0x20)) {} out(0x3f8, *s++); }
    out(0xf4, 0x10);
    for (;;) __asm__ volatile("hlt");
}
""")
    obj = root / "probe.obj"
    efi = root / "BOOTX64.EFI"
    subprocess.run(["clang", "--target=x86_64-pc-windows-msvc", "-ffreestanding",
                    "-fno-stack-protector", "-c", str(c_source), "-o", str(obj)],
                   env={**os.environ, "NIX_HARDENING_ENABLE": "", "NIX_HARDENING_DISABLE": "all"},
                   check=True)
    run(["lld-link", "/subsystem:efi_application", "/entry:efi_main", "/nodefaultlib", f"/out:{efi}", obj])
    fat = source / "efiimg.bin"
    with fat.open("wb") as handle:
        handle.truncate(16 * 1024 * 1024)
    run(["mformat", "-i", fat, "::"])
    run(["mmd", "-i", fat, "::/EFI", "::/EFI/BOOT"])
    run(["mcopy", "-i", fat, efi, "::/EFI/BOOT/BOOTX64.EFI"])
    grub_config = root / "grub.cfg"
    grub_config.write_text("serial --unit=0 --speed=115200\nterminal_output serial\n"
                           "search --file --set=root /proof.txt\ncat /proof.txt\nhalt\n")
    (source / "proof.txt").write_text("FIRMWARE_BOOT_PASS_GRUB\n")
    run(["grub-mkimage", "-d", args.grub, "-O", "i386-pc-eltorito", "-o", source / "grub.bin",
         "-c", grub_config, "-p", "/boot/grub", "biosdisk", "iso9660", "serial", "terminal",
         "cat", "search", "search_fs_file", "halt"])
    grub_image = source / "grub.bin"
    with grub_image.open("ab") as handle:
        handle.write(bytes(-grub_image.stat().st_size % 512))
    (root / "grub-mbr.bin").write_bytes((args.grub / "boot_hybrid.img").read_bytes()[:440])
    run(["cargo", "run", "--locked", "-p", "libmkiso", "--example", "firmware_images", "--", root])
    qemu = shutil.which("qemu-system-x86_64")
    if not qemu:
        raise RuntimeError("QEMU unavailable")
    code = args.firmware / "OVMF_CODE.fd"
    variables = args.firmware / "OVMF_VARS.fd"
    bios = args.bios or Path(qemu).resolve().parent.parent / "share/qemu/bios-256k.bin"
    if not bios.is_file():
        raise RuntimeError(f"SeaBIOS firmware unavailable: {bios}")
    report = {"qemu_version": subprocess.check_output([qemu, "--version"], text=True),
              "tool_versions": {tool: subprocess.check_output([tool, "--version"], text=True).splitlines()[0]
                                for tool in ["nasm", "clang", "lld-link", "grub-mkimage"]},
              "firmware_sha256": sha(code), "pristine_vars_sha256": sha(variables),
              "seabios_sha256": sha(bios),
              "harness_sha256": sha(__file__),
              "fixture_generator_sha256": sha("examples/firmware_images.rs"),
              "source_sha256": {path.name: sha(path) for path in sorted(source.iterdir())},
              "bootstrap_sha256": {name: sha(root / name) for name in ["isohdpfx.bin", "grub-mbr.bin"]},
              "secure_boot": False, "cases": []}
    cases = [(name, firmware, medium, "BIOS" if firmware == "bios" else "UEFI")
             for name in ["level1-mbr", "level2-gpt", "level3-mbr-gpt"]
             for firmware in ["bios", "uefi"] for medium in ["cd", "usb"]]
    cases += [(name, "bios", "cd", "BIOS")
              for name in ["bios-no-emulation", "bios-floppy", "bios-hard-disk"]]
    cases += [("grub-hybrid", "bios", medium, "GRUB") for medium in ["cd", "usb"]]
    # No boot media is a negative control for stale/log-only false positives.
    cases += [("negative-no-media", "bios", "none", "BIOS")]
    for name, firmware, medium, marker in cases:
        identity = f"{name}-{firmware}-{medium}"
        state = root / identity
        state.mkdir()
        image = root / f"{name}.iso"
        command = [qemu, "-name", identity, "-machine", "q35", "-accel", "tcg",
                   "-m", "256", "-display", "none", "-monitor", "none", "-nic", "none",
                   "-serial", f"file:{state / 'serial.log'}", "-no-reboot",
                   "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]
        if firmware == "uefi":
            fresh_vars = state / "OVMF_VARS.fd"
            shutil.copyfile(variables, fresh_vars)
            command += ["-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={code}",
                        "-drive", f"if=pflash,format=raw,unit=1,file={fresh_vars}"]
        else:
            command += ["-bios", str(bios)]
        if medium == "cd":
            command += ["-drive", f"file={image},format=raw,media=cdrom,readonly=on", "-boot", "d"]
        elif medium == "usb":
            command += ["-device", "qemu-xhci", "-drive", f"if=none,id=media,file={image},format=raw,readonly=on",
                        "-device", "usb-storage,drive=media,bootindex=1"]
        started = time.monotonic()
        with (state / "stderr.log").open("wb") as err:
            process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=err)
            expected = f"FIRMWARE_BOOT_PASS_{marker}".encode()
            observed = False
            try:
                while time.monotonic() - started < (5 if medium == "none" else args.timeout):
                    serial = state / "serial.log"
                    observed = serial.exists() and expected in serial.read_bytes()
                    if observed or process.poll() is not None:
                        break
                    time.sleep(0.2)
            finally:
                running_before_cleanup = process.poll() is None
                if running_before_cleanup:
                    process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        serial = state / "serial.log"
        observed = serial.exists() and expected in serial.read_bytes()
        passed = (not observed and running_before_cleanup and process.returncode in (0, -15)) if medium == "none" else observed
        record = {"case": identity, "passed": passed, "marker_observed": observed,
                  "elapsed_seconds": round(time.monotonic() - started, 2),
                  "returncode": process.returncode, "command": command,
                  "image_sha256": sha(image) if medium != "none" else None}
        report["cases"].append(record)
        (root / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"{'PASS' if passed else 'FAIL'} {identity}", flush=True)
    if not all(case["passed"] for case in report["cases"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
