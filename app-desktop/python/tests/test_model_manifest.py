import json
import tempfile
import unittest
from pathlib import Path

from worker.asr import service as asr_service
from worker.model_manifest import create_manifest, verify_manifest
from worker.tts import service as tts_service


class ModelManifestTests(unittest.TestCase):
    def test_manifest_detects_corrupt_asr_model_without_rewriting_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "small"
            root.mkdir()
            for name in asr_service.MODEL_FILES:
                (root / name).write_bytes(("test:" + name).encode())
            (root / "vocabulary.txt").write_bytes(b"test vocabulary")

            self.assertTrue(asr_service._model_is_complete(root))
            manifest = root / asr_service.MANIFEST_NAME
            before = manifest.read_bytes()
            (root / "model.bin").write_bytes(b"evil:model.bin")

            self.assertFalse(asr_service._model_is_complete(root))
            self.assertEqual(manifest.read_bytes(), before)

    def test_complete_interrupted_asr_install_is_recovered_without_preprocessor_config(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            staging = cache / ".small.installing"
            staging.mkdir()
            for name in asr_service.MODEL_FILES:
                (staging / name).write_bytes(("fixture:" + name).encode())
            (staging / "vocabulary.txt").write_bytes(b"fixture vocabulary")

            self.assertFalse((staging / "preprocessor_config.json").exists())
            self.assertTrue(asr_service._recover_staging_install("small", cache))
            self.assertTrue(asr_service._model_is_complete(cache / "small"))
            self.assertFalse(staging.exists())

    def test_incomplete_asr_staging_is_preserved_for_resume(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            staging = cache / ".small.installing"
            staging.mkdir()
            for name in asr_service.MODEL_FILES:
                (staging / name).write_bytes(("fixture:" + name).encode())

            self.assertFalse(asr_service._recover_staging_install("small", cache))
            self.assertTrue(staging.is_dir())
            self.assertFalse((cache / "small").exists())

    def test_manifest_rejects_path_traversal_entries(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = {
                "schemaVersion": 1,
                "identity": "model",
                "files": [{"path": "../outside.bin", "size": 1, "mtimeNs": 1, "sha256": "0" * 64}],
            }
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps(payload), encoding="utf-8")
            self.assertFalse(verify_manifest(root, manifest, "model"))

    def test_tts_is_not_complete_when_an_onnx_component_is_modified(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model = root / "hub" / "models--vieneu" / "snapshots" / "model-rev"
            codec = root / "hub" / "models--moss" / "snapshots" / "codec-rev"
            model_file = model / "onnx_update" / "vieneu_prefill.onnx"
            codec_file = codec / "moss_audio_tokenizer_decode_full.onnx"
            model_file.parent.mkdir(parents=True)
            codec_file.parent.mkdir(parents=True)
            model_file.write_bytes(b"vieneu model fixture")
            codec_file.write_bytes(b"moss codec fixture")

            complete, _ = tts_service._validate_and_manifest(root, model, codec)
            self.assertTrue(complete)
            model_file.write_bytes(b"corrupted model fixture")
            complete, _ = tts_service._validate_and_manifest(root, model, codec)
            self.assertFalse(complete)

    def test_manifest_creation_requires_files_to_stay_inside_cache(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as outside:
            root = Path(directory)
            external = Path(outside) / "model.bin"
            external.write_bytes(b"not managed")
            with self.assertRaisesRegex(ValueError, "outside its managed cache"):
                create_manifest(
                    root, root / "manifest.json", "model", [external],
                    model_id="model", version="1", download="https://example.invalid/model",
                )


if __name__ == "__main__":
    unittest.main()
