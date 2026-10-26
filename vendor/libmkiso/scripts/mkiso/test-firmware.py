#!/usr/bin/env python3
"""Explicit, bounded firmware observation; screenshots never imply a passed gate."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time


def sha256(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


class Qmp:
    def __init__(self, path, transcript):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(3)
        self.sock.connect(str(path))
        self.reader = self.sock.makefile('rb')
        self.transcript = transcript
        self.read()
        self.command('qmp_capabilities')

    def read(self):
        line = self.reader.readline()
        if not line:
            raise RuntimeError('QMP connection closed')
        value = json.loads(line)
        self.transcript.write(json.dumps({'received': value}) + '\n')
        self.transcript.flush()
        return value

    def command(self, name, arguments=None):
        value = {'execute': name, 'arguments': arguments or {}}
        self.transcript.write(json.dumps({'sent': value}) + '\n')
        self.transcript.flush()
        self.sock.sendall((json.dumps(value) + '\n').encode())
        while True:
            result = self.read()
            if 'error' in result:
                raise RuntimeError(str(result['error']))
            if 'return' in result:
                return result['return']

    def close(self):
        self.reader.close()
        self.sock.close()


def run(args):
    image = args.image.resolve(strict=True)
    if ',' in str(image) or ',' in str(args.output.resolve()):
        raise ValueError('QEMU drive paths must not contain commas')
    if not image.is_file():
        raise ValueError('test image must be a regular file')
    output = args.output.resolve()
    if (output / 'report.json').exists():
        raise ValueError('evidence output must not contain an existing report.json')
    if args.firmware not in ('bios', 'uefi', 'bios,uefi'):
        raise ValueError('firmware must be bios, uefi or bios,uefi')
    if len(args.key) > 100:
        raise ValueError('at most 100 selection keys may be sent')
    protected = [image, *args.backend_asset, args.bios, args.ovmf_code, args.ovmf_vars, args.swtpm]
    if any(path and path.resolve() == output / 'report.json' for path in protected):
        raise ValueError('report.json conflicts with a protected input')
    output.mkdir(parents=True, exist_ok=True)
    report = {'schema_version': 1, 'image': str(image), 'image_sha256': sha256(image),
              'backend': args.backend, 'backend_assets': [], 'cases': [],
              'secure_boot': 'not_tested', 'physical_firmware': 'not_tested',
              'cpus': 2, 'cpu_model': 'host' if args.accel == 'kvm' else 'max',
              'memory_mib': args.memory, 'tpm2': bool(args.swtpm)}
    for asset in args.backend_asset:
        report['backend_assets'].append({'path': str(asset.resolve(strict=True)), 'sha256': sha256(asset)})
    qemu = shutil.which(args.qemu)
    qemu_img = shutil.which(args.qemu_img)
    if not qemu or not qemu_img:
        raise ValueError('qemu-system-x86_64 and qemu-img are required')
    if args.swtpm:
        swtpm = args.swtpm.resolve(strict=True)
        report['swtpm'] = {'path': str(swtpm), 'sha256': sha256(swtpm)}
    report['qemu_sha256'] = sha256(qemu)
    report['qemu_version'] = subprocess.check_output([qemu, '--version'], text=True, timeout=10).splitlines()[0]
    for firmware in args.firmware.split(','):
        if firmware not in ('bios', 'uefi'):
            raise ValueError('firmware must be bios, uefi or bios,uefi')
        case = {'firmware': firmware, 'entry_id': args.entry, 'status': 'failed',
                'menu_selection': 'not_evaluated', 'installer_startup': 'not_evaluated',
                'installation_result': 'not_evaluated', 'payload_discovery': 'not_evaluated',
                'root_discovery': 'not_evaluated', 'keys_sent': [], 'artifacts': {}}
        report['cases'].append(case)
        case_dir = Path(tempfile.mkdtemp(prefix=firmware + '-', dir=output))
        # Socket path deliberately short: output locations may exceed AF_UNIX limits.
        with tempfile.TemporaryDirectory(prefix='mkiso-qmp-') as sockets:
            qmp_path = Path(sockets) / 'qmp'
            overlay = case_dir / 'media.qcow2'
            command = [qemu, '-machine', 'q35', '-accel', args.accel, '-m', str(args.memory),
                       '-cpu', report['cpu_model'], '-smp', '2', '-no-reboot', '-display', 'none', '-monitor', 'none', '-nic', 'none',
                       '-qmp', 'unix:' + str(qmp_path) + ',server=on,wait=off',
                       '-serial', 'file:' + str(case_dir / 'serial.log')]
            process = None
            tpm_process = None
            client = None
            try:
                if args.swtpm:
                    state = case_dir / 'tpm'
                    state.mkdir()
                    tpm_socket = Path(sockets) / 'tpm'
                    tpm_command = [str(swtpm), 'socket', '--tpm2',
                                   '--tpmstate', 'dir=' + str(state),
                                   '--ctrl', 'type=unixio,path=' + str(tpm_socket),
                                   '--flags', 'not-need-init']
                    case['tpm_command'] = tpm_command
                    with open(case_dir / 'swtpm.log', 'wb') as tpm_log:
                        tpm_process = subprocess.Popen(tpm_command, stdout=tpm_log, stderr=tpm_log, start_new_session=True)
                    deadline = time.monotonic() + 10
                    while not tpm_socket.exists():
                        if tpm_process.poll() is not None or time.monotonic() >= deadline:
                            raise RuntimeError('swtpm exited or timed out during startup')
                        time.sleep(0.1)
                    command += ['-chardev', 'socket,id=chrtpm,path=' + str(tpm_socket),
                                '-tpmdev', 'emulator,id=tpm0,chardev=chrtpm',
                                '-device', 'tpm-tis,tpmdev=tpm0']
                if firmware == 'uefi':
                    if not args.ovmf_code or not args.ovmf_vars:
                        raise ValueError('UEFI requires --ovmf-code and pristine --ovmf-vars')
                    code = args.ovmf_code.resolve(strict=True)
                    variables = args.ovmf_vars.resolve(strict=True)
                    if any(',' in str(p) for p in (code, variables)):
                        raise ValueError('QEMU firmware paths must not contain commas')
                    case['firmware_assets'] = [{'path': str(p), 'sha256': sha256(p)} for p in (code, variables)]
                    copied_vars = case_dir / 'OVMF_VARS.fd'
                    shutil.copyfile(variables, copied_vars)
                    command += ['-drive', 'if=pflash,format=raw,readonly=on,file=' + str(code),
                                '-drive', 'if=pflash,format=raw,file=' + str(copied_vars)]
                else:
                    if not args.bios:
                        raise ValueError('BIOS requires --bios firmware file for reproducible hashing')
                    bios = args.bios.resolve(strict=True)
                    case['firmware_assets'] = [{'path': str(bios), 'sha256': sha256(bios)}]
                    command += ['-bios', str(bios)]
                subprocess.run([qemu_img, 'create', '-f', 'qcow2', '-F', 'raw', '-b', str(image), str(overlay)],
                               check=True, stdout=subprocess.DEVNULL, timeout=20)
                if args.media == 'optical':
                    command += ['-drive', 'file=' + str(image) + ',format=raw,media=cdrom,readonly=on', '-boot', 'order=d']
                else:
                    command += ['-drive', 'if=none,id=media,format=qcow2,file=' + str(overlay),
                                '-device', 'qemu-xhci', '-device', 'usb-storage,drive=media,bootindex=1']
                case['command'] = command
                with open(case_dir / 'qemu.log', 'wb') as log, open(case_dir / 'qmp.jsonl', 'w') as transcript:
                    process = subprocess.Popen(command, stdout=log, stderr=log, start_new_session=True)
                    deadline = time.monotonic() + args.timeout
                    while not qmp_path.exists():
                        if process.poll() is not None or time.monotonic() >= deadline:
                            raise RuntimeError('QEMU exited or timed out before QMP startup')
                        time.sleep(0.1)
                    client = Qmp(qmp_path, transcript)
                    start = time.monotonic()
                    selected = False
                    while time.monotonic() < deadline:
                        if process.poll() is not None:
                            raise RuntimeError('QEMU exited during observation')
                        if any(p.exists() and p.stat().st_size > 64 * 1024 * 1024
                               for p in (case_dir / 'qemu.log', case_dir / 'serial.log', case_dir / 'swtpm.log')):
                            raise RuntimeError('firmware log exceeded 64 MiB limit')
                        if not selected and time.monotonic() - start >= args.select_after:
                            client.command('screendump', {'filename': str(case_dir / 'before-selection.ppm')})
                            for key in args.key:
                                client.command('send-key', {'keys': [{'type': 'qcode', 'data': key}], 'hold-time': 100})
                                case['keys_sent'].append(key)
                                time.sleep(0.2)
                            selected = True
                        time.sleep(0.2)
                    client.command('screendump', {'filename': str(case_dir / 'final.ppm')})
                    case['qemu_status'] = client.command('query-status')
                    case['status'] = 'observation_complete'
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
                case['error'] = str(error)
            finally:
                if client:
                    client.close()
                if process and process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=5)
                if tpm_process and tpm_process.poll() is None:
                    os.killpg(tpm_process.pid, signal.SIGTERM)
                    try:
                        tpm_process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(tpm_process.pid, signal.SIGKILL)
                        tpm_process.wait(timeout=5)
                if args.swtpm:
                    case['tpm_state'] = [{'path': str(p), 'sha256': sha256(p)}
                                         for p in sorted((case_dir / 'tpm').glob('*')) if p.is_file()]
                for artifact in case_dir.iterdir():
                    if artifact.is_file() and artifact.name not in ('media.qcow2', 'OVMF_VARS.fd'):
                        case['artifacts'][artifact.name] = {'path': str(artifact), 'sha256': sha256(artifact)}
                report['image_sha256_after'] = sha256(image)
                if report['image_sha256_after'] != report['image_sha256']:
                    case['status'] = 'failed'
                    case['error'] = 'source image changed during observation'
                (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))
    return 0 if all(c['status'] == 'observation_complete' for c in report['cases']) else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('image', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--firmware', default='bios,uefi')
    parser.add_argument('--media', choices=['usb', 'optical'], default='usb')
    parser.add_argument('--entry', required=True)
    parser.add_argument('--backend', required=True)
    parser.add_argument('--backend-asset', type=Path, action='append', default=[])
    parser.add_argument('--bios', type=Path)
    parser.add_argument('--swtpm', type=Path, help='explicit swtpm binary for disposable TPM 2.0')
    parser.add_argument('--ovmf-code', type=Path, default=os.environ.get('OVMF_CODE'))
    parser.add_argument('--ovmf-vars', type=Path, default=os.environ.get('OVMF_VARS'))
    parser.add_argument('--key', action='append', default=[])
    parser.add_argument('--select-after', type=float, default=10)
    parser.add_argument('--timeout', type=float, default=30)
    parser.add_argument('--memory', type=int, default=2048)
    parser.add_argument('--accel', choices=['tcg', 'kvm'], default='tcg')
    parser.add_argument('--qemu', default='qemu-system-x86_64')
    parser.add_argument('--qemu-img', default='qemu-img')
    args = parser.parse_args()
    if not 1 <= args.timeout <= 3600 or not 0 <= args.select_after < args.timeout:
        parser.error('timeout must be 1..3600 seconds and select-after must be less than timeout')
    if not 128 <= args.memory <= 65536:
        parser.error('memory must be 128..65536 MiB')
    def interrupted(signum, frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    try:
        return run(args)
    except KeyboardInterrupt:
        parser.exit(130, 'firmware observation cancelled; evidence retained\n')
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        parser.exit(2, str(error) + '\n')


if __name__ == '__main__':
    raise SystemExit(main())
