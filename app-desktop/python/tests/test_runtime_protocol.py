"""These tests intentionally run without any third-party AI/browser packages."""
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]


class IsolatedProtocolTests(unittest.TestCase):
    def run_worker(self, kind, lines):
        return subprocess.run([sys.executable, '-S', '-m', kind], input=lines,
            encoding='utf-8', capture_output=True, timeout=15,
            env={**os.environ, 'PYTHONPATH': str(ROOT), 'PYTHONUTF8': '1'}, cwd=ROOT)

    def test_ping_health_version_need_no_dependencies(self):
        for kind in ['worker', 'downloader_worker']:
            with self.subTest(kind=kind):
                result = self.run_worker(kind, '\n'.join(json.dumps({'operation': operation, 'requestId': operation})
                    for operation in ['ping', 'health', 'version']) + '\n')
                self.assertEqual(result.returncode, 0, result.stderr)
                replies = [json.loads(line) for line in result.stdout.splitlines()]
                self.assertEqual([r['requestId'] for r in replies], ['ping', 'health', 'version'])
                self.assertTrue(all(r['type'] == 'ready' and r['protocolVersion'] == 2 for r in replies))
                self.assertEqual(replies[0]['workerKind'], 'ai' if kind == 'worker' else 'downloader')

    def test_modules_import_without_models_or_browser_sdk(self):
        script = "import worker.__main__,downloader_worker.__main__,sys; print(','.join(sys.modules))"
        result = subprocess.run([sys.executable, '-S', '-c', script], env={**os.environ, 'PYTHONPATH': str(ROOT)},
            capture_output=True, encoding='utf-8', timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        modules = result.stdout.split(',')
        for excluded in ['torch', 'numpy', 'vieneu', 'rapidocr', 'faster_whisper', 'playwright', 'curl_cffi',
                         'worker.asr.service', 'worker.tts.service', 'downloader_worker.tiktok_drama.service']:
            self.assertNotIn(excluded, modules)

    def test_invalid_json_version_and_shape_do_not_poison_next_request(self):
        result = self.run_worker('worker', 'broken\n[]\n{"protocolVersion":1,"operation":"ping"}\n{"operation":"ping"}\n')
        replies = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual([r['type'] for r in replies], ['failed', 'failed', 'failed', 'ready'])
        self.assertEqual(replies[2]['code'], 'PROTOCOL_MISMATCH')

    def test_request_id_unicode_and_logs_do_not_expose_payload(self):
        secret = 'private-cookie-token'
        result = self.run_worker('worker', json.dumps({'operation': 'unknown', 'cookie': secret, 'requestId': 'Tiếng Việt 😀'}) + '\n')
        reply = json.loads(result.stdout)
        self.assertEqual(reply['requestId'], 'Tiếng Việt 😀')
        self.assertNotIn(secret, result.stdout + result.stderr)

    def test_shutdown_stops_before_later_request(self):
        result = self.run_worker('downloader_worker', '{"operation":"shutdown"}\n{"operation":"ping"}\n')
        self.assertEqual(len(result.stdout.splitlines()), 1)
        self.assertEqual(json.loads(result.stdout)['status'], 'stopped')

    def test_ai_rejects_download_and_downloader_rejects_ai(self):
        for kind, operation in [('worker', 'tiktok_download'), ('downloader_worker', 'transcribe')]:
            result = self.run_worker(kind, json.dumps({'operation': operation}) + '\n')
            self.assertEqual(json.loads(result.stdout)['type'], 'failed')

    def test_missing_downloader_dependency_does_not_affect_ai(self):
        result = self.run_worker('downloader_worker', '{"operation":"downloader_inspect"}\n')
        self.assertEqual(json.loads(result.stdout)['type'], 'failed')
        self.assertEqual(json.loads(result.stdout)['code'], 'WORKER_ERROR')
        self.assertEqual(json.loads(self.run_worker('worker', '{"operation":"ping"}\n').stdout)['type'], 'ready')

    def test_oversize_input_is_bounded_and_worker_stops(self):
        result = self.run_worker('worker', 'x' * (8 * 1024 * 1024 + 1) + '\n')
        self.assertEqual(json.loads(result.stdout)['code'], 'REQUEST_TOO_LARGE')


if __name__ == '__main__':
    unittest.main()
