"""Regression tests for evidence protocol and fail-closed gate reporting."""
import importlib.util
import io
import json
from pathlib import Path
import socket
import tempfile
import threading
import unittest

SPEC = importlib.util.spec_from_file_location('firmware_harness', Path(__file__).with_name('test-firmware.py'))
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)


class FirmwareHarnessTests(unittest.TestCase):
    def test_qmp_events_do_not_count_as_command_success(self):
        with tempfile.TemporaryDirectory() as directory:
            address = Path(directory) / 'qmp'
            listener = socket.socket(socket.AF_UNIX)
            listener.bind(str(address))
            listener.listen(1)
            def server():
                connection, _ = listener.accept()
                with connection, connection.makefile('rb') as reader:
                    connection.sendall(b'{"QMP": {}}\n')
                    reader.readline()
                    connection.sendall(b'{"event": "RESET"}\n{"return": {}}\n')
                    request = json.loads(reader.readline())
                    self.assertEqual(request['execute'], 'screendump')
                    connection.sendall(b'{"error": {"desc": "capture failed"}}\n')
            worker = threading.Thread(target=server)
            worker.start()
            transcript = io.StringIO()
            client = HARNESS.Qmp(address, transcript)
            try:
                with self.assertRaisesRegex(RuntimeError, 'capture failed'):
                    client.command('screendump', {'filename': 'capture.ppm'})
            finally:
                client.close()
                worker.join(timeout=5)
                listener.close()
            self.assertFalse(worker.is_alive())
            self.assertIn('RESET', transcript.getvalue())
            self.assertIn('capture failed', transcript.getvalue())

    def test_hash_reads_full_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'payload'
            path.write_bytes(b'abc')
            self.assertEqual(HARNESS.sha256(path), 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')


if __name__ == '__main__':
    unittest.main()
