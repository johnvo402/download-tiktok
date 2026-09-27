import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("worker_cache", Path(__file__).resolve().parents[1] / "worker-cache.py")
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)


class WorkerCacheTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="jc-worker-cache-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.inputs = ["python/worker/__main__.py", "python/worker/_version.py", "python/packaging/worker.spec",
                       "python/pyproject.toml", "python/requirements-release.txt", "scripts/build-worker.ps1", "scripts/worker-cache.py"]
        for name in self.inputs:
            file = self.root / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text("initial")

    def test_key_changes_for_every_build_input_and_interpreter(self):
        key = cache.fingerprint(self.root, "python-one")
        for name in self.inputs:
            with self.subTest(name=name):
                file = self.root / name
                file.write_text("changed")
                self.assertNotEqual(key, cache.fingerprint(self.root, "python-one"))
                file.write_text("initial")
        self.assertNotEqual(key, cache.fingerprint(self.root, "python-two"))

    def test_app_version_and_bytecode_do_not_change_key(self):
        key = cache.fingerprint(self.root, "python-one")
        (self.root / "package.json").write_text('{"version":"2.0.0"}')
        bytecode = self.root / "python/worker/__pycache__/junk.pyc"
        bytecode.parent.mkdir()
        bytecode.write_bytes(b"different machines produce different bytecode")
        self.assertEqual(key, cache.fingerprint(self.root, "python-one"))

    def test_missing_cache_mismatched_key_changed_missing_and_extra_files(self):
        output = self.root / "output"
        output.mkdir()
        self.assertFalse(cache.verify(output, "key"))
        for name in ["ai-worker.exe", "worker-version.txt", "python-packages.json", "THIRD_PARTY_NOTICES.txt", "runtime.dll"]:
            (output / name).write_text("original")
        manifest = {"fingerprint": "key", "files": cache.inventory(output)}
        (output / cache.MANIFEST).write_text(json.dumps(manifest))
        self.assertTrue(cache.verify(output, "key"))
        self.assertFalse(cache.verify(output, "different"))
        (output / "runtime.dll").write_text("corrupt")
        self.assertFalse(cache.verify(output, "key"))
        (output / "runtime.dll").unlink()
        self.assertFalse(cache.verify(output, "key"))
        (output / "runtime.dll").write_text("original")
        (output / "unexpected.dll").write_text("unexpected")
        self.assertFalse(cache.verify(output, "key"))


if __name__ == "__main__":
    unittest.main()
