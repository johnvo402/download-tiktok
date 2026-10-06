import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from worker.model_manifest import create_manifest, verify_manifest
from worker.tts import service
from worker.tts.onnx_cache import materialize_onnx_files


class TtsOnnxCacheTests(unittest.TestCase):
    def make_snapshots(self, root):
        model = root / "hub/models--vieneu/snapshots/model-rev"
        codec = root / "hub/models--moss/snapshots/codec-rev"
        contents = {
            model / "onnx_update/vieneu_prefill.onnx": b"graph",
            model / "onnx_update/vieneu_backbone_shared.data": b"weights",
            codec / "moss_audio_tokenizer_decode_full.onnx": b"codec graph",
            codec / "moss_audio_tokenizer_decode_shared.data": b"codec weights",
        }
        for index, (file_path, content) in enumerate(contents.items()):
            blob = root / f"hub/blobs/{index:02d}/content"
            blob.parent.mkdir(parents=True)
            blob.write_bytes(content)
            file_path.parent.mkdir(parents=True, exist_ok=True)
            file_path.symlink_to(blob)
        return model, codec, contents

    def test_graphs_and_weights_stay_in_the_same_canonical_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model, codec, contents = self.make_snapshots(root)
            blobs = {path: path.resolve() for path in contents}
            self.assertTrue(materialize_onnx_files(root, model, codec))
            for path, content in contents.items():
                self.assertFalse(path.is_symlink())
                self.assertEqual(path.resolve().parent, path.parent.resolve())
                self.assertEqual(path.read_bytes(), content)
                self.assertTrue(os.path.samefile(path, blobs[path]))
                self.assertEqual(blobs[path].read_bytes(), content)
            self.assertFalse(materialize_onnx_files(root, model, codec))

    def test_filesystems_without_hard_links_use_regular_copies(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model, codec, contents = self.make_snapshots(root)
            blobs = {path: path.resolve() for path in contents}
            with patch("worker.tts.onnx_cache.os.link", side_effect=OSError("no hard links")):
                materialize_onnx_files(root, model, codec)
            for path, content in contents.items():
                self.assertFalse(path.is_symlink())
                self.assertEqual(path.read_bytes(), content)
                self.assertFalse(os.path.samefile(path, blobs[path]))
            self.assertEqual(list(root.rglob("*.tmp")), [])

    def test_outside_cache_target_is_rejected_before_replacing_any_links(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as outside:
            root = Path(directory)
            model, codec, contents = self.make_snapshots(root)
            foreign = Path(outside) / "weights.data"
            foreign.write_bytes(b"unmanaged")
            extra = codec / "outside.data"
            extra.symlink_to(foreign)
            with self.assertRaises(ValueError):
                materialize_onnx_files(root, model, codec)
            self.assertTrue(all(path.is_symlink() for path in contents))

    def test_failed_copy_preserves_the_original_link(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model, codec, contents = self.make_snapshots(root)
            with patch("worker.tts.onnx_cache.os.link", side_effect=OSError("no hard links")), patch(
                "worker.tts.onnx_cache.shutil.copy2", side_effect=OSError("disk full")
            ):
                with self.assertRaisesRegex(OSError, "disk full"):
                    materialize_onnx_files(root, model, codec)
            self.assertTrue(all(path.is_symlink() for path in contents))
            self.assertEqual(list(root.rglob("*.tmp")), [])

    def test_legacy_manifest_migrates_and_recovers_after_partial_materialization(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model, codec, _ = self.make_snapshots(root)
            identity = f"{service.MODEL_REPO}@{model.name}|{service.CODEC_REPO}@{codec.name}"
            manifest = root / service.MANIFEST_NAME
            create_manifest(
                root, manifest, identity, service._onnx_files(model, codec),
                model_id="tts", version="legacy", download="https://example.invalid/model",
            )
            before = manifest.read_bytes()
            materialize_onnx_files(root, model)
            self.assertEqual(service._validate_and_manifest(root, model, codec), (True, "model-rev+codec-rev"))
            self.assertNotEqual(manifest.read_bytes(), before)
            self.assertTrue(verify_manifest(root, manifest, identity))
            self.assertTrue(all("/snapshots/" in entry["path"] for entry in json.loads(manifest.read_text())["files"]))
            self.assertEqual(service._validate_and_manifest(root, model, codec), (True, "model-rev+codec-rev"))

    def test_layout_migration_does_not_accept_a_corrupted_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model, codec, _ = self.make_snapshots(root)
            identity = f"{service.MODEL_REPO}@{model.name}|{service.CODEC_REPO}@{codec.name}"
            manifest = root / service.MANIFEST_NAME
            create_manifest(
                root, manifest, identity, service._onnx_files(model, codec),
                model_id="tts", version="legacy", download="https://example.invalid/model",
            )
            before = manifest.read_bytes()
            graph = model / "onnx_update/vieneu_prefill.onnx"
            graph.unlink()
            graph.write_bytes(b"bad graph")
            self.assertEqual(service._validate_and_manifest(root, model, codec), (False, None))
            self.assertEqual(manifest.read_bytes(), before)
            self.assertTrue((codec / "moss_audio_tokenizer_decode_full.onnx").is_symlink())


if __name__ == "__main__":
    unittest.main()
