import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

WORKER_ROOT = Path(__file__).resolve().parents[1]


class WorkerProtocolTests(unittest.TestCase):
    def test_unknown_operation_returns_json_error_on_stdout(self):
        environment = os.environ.copy()
        environment["PYTHONPATH"] = str(WORKER_ROOT)
        result = subprocess.run(
            [sys.executable, "-m", "worker"],
            input='{"operation":"unknown"}\n',
            text=True,
            capture_output=True,
            cwd=WORKER_ROOT,
            env=environment,
            check=False,
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout.splitlines()[0])["type"], "failed")
        self.assertIn("Unsupported worker operation", result.stderr)


if __name__ == "__main__":
    unittest.main()
