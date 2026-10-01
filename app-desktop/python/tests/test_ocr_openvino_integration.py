"""Real Intel tests: opt-in, cached models only, no network/model downloads."""
import os
import shutil
import gc
import time
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


@unittest.skipUnless(os.environ.get("JC_AI_TEST_OPENVINO") == "1", "Intel/model tests are opt-in")
class IntelOcrModelTests(unittest.TestCase):
    def setUp(self):
        from worker.ocr.backends import probe_openvino_gpu
        from worker.ocr.service import _model_root, _model_files, MODEL_PROFILES
        cap = probe_openvino_gpu()
        if not cap.available:
            self.skipTest(cap.message)
        source = os.environ.get("JC_AI_TEST_OCR_MODELS")
        if not source:
            self.skipTest("Set JC_AI_TEST_OCR_MODELS to an installed models directory")
        files = _model_files(_model_root(source, "tiny"))
        names = MODEL_PROFILES["tiny"]["requiredFiles"]
        if not all(name in files for name in names):
            self.skipTest("Tiny weights not installed; tests never download them")
        temporary = tempfile.TemporaryDirectory(prefix="jc-intel-ocr-", ignore_cleanup_errors=True)
        def cleanup():
            # Intel's dynamic-shape kernel cache can finish an asynchronous disk
            # write just after inference. Release models before deleting fixtures.
            gc.collect()
            for attempt in range(20):
                temporary.cleanup()
                if not Path(temporary.name).exists():
                    return
                time.sleep(0.1)
            self.fail("Intel fixture cache still busy after cleanup")
        self.addCleanup(cleanup)
        self.root = Path(temporary.name)
        self.models = self.root / "models"
        target = self.models / "rapidocr" / "tiny"
        target.mkdir(parents=True)
        for name in names:
            shutil.copyfile(files[name], target / name)
        from PIL import Image, ImageDraw, ImageFont
        self.image = Image.new("RGB", (640, 160), "white")
        try:
            font = ImageFont.truetype("arial.ttf", 48)
        except OSError:
            font = ImageFont.load_default(size=48)
        ImageDraw.Draw(self.image).text((20, 40), "Hello world 12345", fill="black", font=font)

    def test_same_real_image_contract_and_models_on_cpu_and_intel(self):
        import numpy as np
        from worker.ocr.service import _create_engine, _extract_text, verify_engine_device
        from worker.text_removal.service import _result_boxes
        paths_before = set(self.models.rglob("*.onnx"))
        texts = []
        for backend in ("cpu", "openvino"):
            engine = _create_engine(str(self.models), "tiny", backend)
            result = engine(np.asarray(self.image))
            sessions = verify_engine_device(engine, backend, "integration")
            self.assertEqual(len(sessions), 3)
            text = _extract_text(result)
            self.assertTrue(text)
            texts.append(text)
            self.assertGreater(len(_result_boxes(result)), 0)
        self.assertEqual(texts[0], texts[1])
        self.assertEqual(set(self.models.rglob("*.onnx")), paths_before)

    def test_real_auto_uses_intel_without_cuda(self):
        import numpy as np
        from worker.ocr.backends import runtime_capabilities
        from worker.ocr.service import _create_engine, verify_engine_device
        if runtime_capabilities()["cuda"].available:
            self.skipTest("Auto correctly prefers CUDA on this machine")
        engine = _create_engine(str(self.models), "tiny", "auto")
        engine(np.asarray(self.image))
        self.assertEqual(engine.resolved_backend, "openvino")
        sessions = verify_engine_device(engine, "openvino", "auto-integration")
        self.assertTrue(all(devices and all(d.startswith("GPU") for d in devices) for devices in sessions.values()))

    def test_real_transcript_loop_deduplicates_and_emits_progress(self):
        import json
        from worker.ocr.service import transcribe_frames
        frames = self.root / "frames"
        frames.mkdir()
        for index in range(1, 4):
            self.image.save(frames / f"frame_{index:06d}.jpg")
        output = self.root / "transcript.json"
        events = []
        transcribe_frames({"framesDir": str(frames), "outputPath": str(output), "cacheDir": str(self.models),
                           "model": "tiny", "device": "openvino", "fps": 1.0}, events.append)
        transcript = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(len(transcript["segments"]), 1)
        self.assertEqual(transcript["segments"][0]["start"], 0.0)
        self.assertEqual(transcript["segments"][0]["end"], 3.0)
        self.assertTrue(any(event["type"] == "progress" for event in events))
        self.assertEqual(events[-1]["type"], "completed")


if __name__ == "__main__":
    unittest.main()
