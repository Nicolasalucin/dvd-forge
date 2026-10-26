#!/usr/bin/env python3
"""Run an explicitly requested installer case; retain timed screenshots and QMP."""
import argparse
import importlib.util
import json
import os
import signal
import shutil
from pathlib import Path
import subprocess
import tempfile
import time

spec = importlib.util.spec_from_file_location("firmware", Path(__file__).with_name("test-firmware.py"))
firmware = importlib.util.module_from_spec(spec)
spec.loader.exec_module(firmware)


def qcodes(character):
    if character.isalpha():
        return (["shift"] if character.isupper() else []) + [character.lower()]
    if character.isdigit():
        return [character]
    symbols = {" ": ["spc"], ":": ["shift", "semicolon"], ";": ["semicolon"],
               "\\": ["backslash"], "/": ["slash"], "-": ["minus"],
               "_": ["shift", "minus"], ".": ["dot"], "*": ["shift", "8"],
               "=": ["equal"], "%": ["shift", "5"], "(": ["shift", "9"],
               ")": ["shift", "0"], "@": ["shift", "2"], ",": ["comma"], "\"": ["shift", "apostrophe"], "\n": ["ret"]}
    if character not in symbols:
        raise ValueError(f"unsupported keyboard character: {character!r}")
    return symbols[character]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--entry", required=True)
    parser.add_argument("--backend-asset", type=Path)
    parser.add_argument("--firmware", choices=["bios", "uefi"], required=True)
    parser.add_argument("--media", choices=["usb", "optical"], default="usb")
    parser.add_argument("--bios", type=Path)
    parser.add_argument("--ovmf-code", type=Path)
    parser.add_argument("--ovmf-vars", type=Path)
    parser.add_argument("--actions", type=Path, required=True,
                        help='JSON list of {at: seconds, keys: [qcodes]} or {at: seconds, text: string}')
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--memory", type=int, default=4096)
    parser.add_argument("--cpu", default="host", help="explicit KVM CPU model, default host")
    parser.add_argument("--swtpm", default=None, help="optional swtpm executable for private TPM 2.0")
    parser.add_argument("--qemu", default="qemu-system-x86_64")
    parser.add_argument("--qemu-img", default="qemu-img")
    args = parser.parse_args()
    if not 10 <= args.timeout <= 900 or not 512 <= args.memory <= 16384:
        parser.error("timeout must be 10..900; memory must be 512..16384 MiB")
    if args.media == "usb" and args.backend_asset is None:
        parser.error("USB cases require the pinned --backend-asset")
    image = args.image.resolve(strict=True)
    output = args.output.resolve()
    if not image.is_file() or "," in str(image) or "," in str(output):
        parser.error("image must be regular; QEMU paths cannot contain commas")
    output.mkdir(parents=True, exist_ok=False)
    actions = json.loads(args.actions.read_text())
    if not isinstance(actions, list):
        parser.error("actions must be a JSON list")
    for action in actions:
        if set(action) not in ({"at", "keys"}, {"at", "text"}) or not 0 <= action["at"] < args.timeout:
            parser.error("invalid action keys or timing")
        for character in action.get("text", ""):
            qcodes(character)
    actions.sort(key=lambda action: action["at"])
    report = {"runner_sha256": firmware.sha256(Path(__file__)), "actions_sha256": firmware.sha256(args.actions), "schema": 1, "entry": args.entry, "firmware": args.firmware,
              "image": str(image), "image_sha256": firmware.sha256(image),
              "secure_boot": "not_tested", "observations": {stage: "not_evaluated" for stage in
                  ("menu_selection", "installer_startup", "payload_discovery", "root_discovery", "installation_result")}, "artifacts": {}}
    qemu_path = Path(shutil.which(args.qemu) or args.qemu).resolve(strict=True)
    report["qemu"] = {"path": str(qemu_path), "sha256": firmware.sha256(qemu_path),
                      "version": subprocess.check_output([str(qemu_path), "--version"], text=True, timeout=10).splitlines()[0]}
    if args.backend_asset:
        report["backend_asset"] = {"path": str(args.backend_asset.resolve(strict=True)),
                                   "sha256": firmware.sha256(args.backend_asset)}
    overlay = output / "media.qcow2"
    if args.media == "usb":
        subprocess.run([args.qemu_img, "create", "-f", "qcow2", "-F", "raw", "-b", str(image), str(overlay)], check=True)
    subprocess.run([args.qemu_img, "create", "-f", "qcow2", str(output / "target.qcow2"), "64G"], check=True)
    process = client = tpm_process = None
    try:
        with tempfile.TemporaryDirectory(prefix="mkiso-case-") as sockets:
            sock = Path(sockets) / "qmp"
            cmd = [args.qemu, "-machine", "q35", "-accel", "kvm", "-m", str(args.memory),
                   "-smp", "2", "-cpu", args.cpu, "-display", "none", "-monitor", "none", "-nic", "none",
                   "-no-reboot", "-qmp", f"unix:{sock},server=on,wait=off",
                   "-serial", "file:" + str(output / "serial.log"),
                   "-drive", "if=none,id=target,format=qcow2,file=" + str(output / "target.qcow2"),
                   "-device", "ide-hd,drive=target,bus=ide.0"]
            if args.media == "usb":
                cmd += ["-drive", "if=none,id=media,format=qcow2,file=" + str(overlay),
                        "-device", "qemu-xhci", "-device", "usb-storage,drive=media,bootindex=1"]
            else:
                cmd += ["-drive", "if=none,id=original,format=raw,readonly=on,file=" + str(image),
                        "-device", "ide-cd,drive=original,bus=ide.1,bootindex=1"]
            report["media"] = args.media
            assets = [args.bios] if args.firmware == "bios" else [args.ovmf_code, args.ovmf_vars]
            if any(asset is None for asset in assets):
                parser.error("selected firmware assets are required")
            report["firmware_assets"] = [{"path": str(p.resolve(strict=True)), "sha256": firmware.sha256(p)} for p in assets]
            if args.firmware == "bios":
                cmd += ["-bios", str(args.bios)]
            else:
                copied = output / "OVMF_VARS.fd"
                copied.write_bytes(args.ovmf_vars.read_bytes())
                cmd += ["-drive", "if=pflash,format=raw,readonly=on,file=" + str(args.ovmf_code),
                        "-drive", "if=pflash,format=raw,file=" + str(copied)]
            if args.swtpm:
                state = output / "tpm"
                state.mkdir()
                tpm_socket = Path(sockets) / "tpm"
                tpm_cmd = [args.swtpm, "socket", "--tpm2", "--tpmstate", "dir=" + str(state),
                           "--ctrl", "type=unixio,path=" + str(tpm_socket), "--flags", "not-need-init"]
                report["swtpm"] = {"command": tpm_cmd, "sha256": firmware.sha256(args.swtpm)}
                with (output / "swtpm.log").open("wb") as tpm_log:
                    tpm_process = subprocess.Popen(tpm_cmd, stdout=tpm_log, stderr=tpm_log, start_new_session=True)
                deadline = time.monotonic() + 10
                while not tpm_socket.exists():
                    if tpm_process.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError("swtpm did not start")
                    time.sleep(.1)
                cmd += ["-chardev", "socket,id=chrtpm,path=" + str(tpm_socket),
                        "-tpmdev", "emulator,id=tpm0,chardev=chrtpm", "-device", "tpm-tis,tpmdev=tpm0"]
            report["command"] = cmd
            with (output / "qemu.log").open("wb") as log, (output / "qmp.jsonl").open("w") as transcript:
                process = subprocess.Popen(cmd, stdout=log, stderr=log)
                start = time.monotonic()
                while not sock.exists():
                    if process.poll() is not None or time.monotonic() - start > 10:
                        raise RuntimeError("QEMU did not start")
                    time.sleep(.1)
                client = firmware.Qmp(sock, transcript)
                cursor = 0
                next_shot = 1
                while time.monotonic() - start < args.timeout:
                    if process.poll() is not None:
                        raise RuntimeError("QEMU exited")
                    elapsed = time.monotonic() - start
                    while cursor < len(actions) and elapsed >= actions[cursor]["at"]:
                        action = actions[cursor]
                        sequences = [action["keys"]] if "keys" in action else [qcodes(c) for c in action["text"]]
                        for keys in sequences:
                            client.command("send-key", {"keys": [{"type": "qcode", "data": k} for k in keys], "hold-time": 60})
                            time.sleep(.09)
                        cursor += 1
                    if elapsed >= next_shot:
                        client.command("screendump", {"filename": str(output / f"screen-{int(next_shot):03}.ppm")})
                        next_shot = next_shot + 1 if next_shot < 10 else next_shot + 10
                    time.sleep(.1)
                client.command("screendump", {"filename": str(output / "final.ppm")})
                report["qemu_status"] = client.command("query-status")
                report["registers"] = client.command("human-monitor-command", {"command-line": "info registers"})
                report["status"] = "observation_complete"
    except Exception as error:
        report["status"] = "failed"
        report["error"] = str(error)
    finally:
        if client:
            client.close()
        if process and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        if tpm_process and tpm_process.poll() is None:
            os.killpg(tpm_process.pid, signal.SIGTERM)
            try:
                tpm_process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(tpm_process.pid, signal.SIGKILL)
                tpm_process.wait(timeout=5)
        report["image_sha256_after"] = firmware.sha256(image)
        if report["image_sha256_after"] != report["image_sha256"]:
            report["status"] = "failed"
            report["error"] = "original image changed"
        for path in output.rglob("*"):
            if path.is_file() and path.suffix not in (".qcow2", ".fd"):
                report["artifacts"][str(path.relative_to(output))] = {"path": str(path), "sha256": firmware.sha256(path)}
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"status": report["status"], "report": str(output / "report.json")}))
    return 0 if report["status"] == "observation_complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
