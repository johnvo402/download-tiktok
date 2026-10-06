import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch

from worker.tts import service


class TtsWorkerTests(unittest.TestCase):
    def test_inspect_without_optional_package_does_not_touch_model_cache(self):
        with patch.object(service.importlib.util, "find_spec", return_value=None), patch.object(
            service, "_local_model_info", side_effect=AssertionError("should not scan cache")
        ):
            result = service.inspect_tts("unused")

        self.assertFalse(result["workerInstalled"])
        self.assertFalse(result["modelCached"])
        self.assertIsNone(result["modelRevision"])
        self.assertEqual(result["voices"], [])

    def test_synthesize_preserves_unicode_and_reference_audio(self):
        captured = {}

        class FakeVieneu:
            sample_rate = 48_000

            def __init__(self, **kwargs):
                captured["init"] = kwargs

            def infer(self, text, **kwargs):
                captured["text"] = text
                captured["infer"] = kwargs
                return [0.0] * 48_000

            def save(self, audio, path):
                Path(path).write_bytes(b"RIFF fake wav")

        fake_module = types.ModuleType("vieneu")
        fake_module.Vieneu = FakeVieneu
        events = []
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reference = root / "voice.wav"
            reference.write_bytes(b"reference")
            output = root / "audio.wav"
            request = {
                "cacheDir": str(root / "cache"),
                "device": "cpu",
                "segments": [
                    {
                        "id": "segment-1",
                        "text": "Xin chào, tiếng Việt có dấu.",
                        "outputPath": str(output),
                        "referenceAudio": str(reference),
                    }
                ],
            }
            with patch.object(service.importlib.util, "find_spec", return_value=object()), patch(
                "worker.tts.service._cuda_available", return_value=False
            ), patch.object(service, "_local_model_info", return_value=(True, "test")), patch.dict(sys.modules, {"vieneu": fake_module}):
                service.synthesize(request, events.append)

            self.assertTrue(output.is_file())

        self.assertEqual(captured["text"], "Xin chào, tiếng Việt có dấu.")
        self.assertEqual(captured["infer"]["ref_audio"], str(reference.resolve()))
        self.assertTrue(any(event["type"] == "segment_completed" for event in events))

    def test_cuda_request_fails_cleanly_when_cuda_is_unavailable(self):
        request = {"cacheDir": ".", "device": "cuda", "segments": [{"id": "a", "text": "Xin chào", "outputPath": "out.wav"}]}
        with patch.object(service.importlib.util, "find_spec", return_value=object()), patch(
            "worker.tts.service._cuda_available", return_value=False
        ):
            with self.assertRaisesRegex(RuntimeError, "CUDA chưa khả dụng"):
                service.synthesize(request, lambda _event: None)

    def test_provider_failure_is_reported_per_segment_without_claiming_success(self):
        class BrokenVieneu:
            sample_rate = 48_000

            def __init__(self, **_kwargs):
                pass

            def infer(self, _text, **_kwargs):
                raise RuntimeError("model load failed")

        fake_module = types.ModuleType("vieneu")
        fake_module.Vieneu = BrokenVieneu
        events = []
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "audio.wav"
            request = {"cacheDir": directory, "device": "cpu", "segments": [{"id": "a", "text": "Chào bạn", "outputPath": str(output)}]}
            with patch.object(service.importlib.util, "find_spec", return_value=object()), patch.object(
                service, "_local_model_info", return_value=(True, "test")
            ), patch.dict(sys.modules, {"vieneu": fake_module}):
                service.synthesize(request, events.append)
            self.assertFalse(output.exists())
        self.assertTrue(any(event["type"] == "segment_failed" and "model load failed" in event["message"] for event in events))
        self.assertFalse(any(event["type"] == "segment_completed" for event in events))

    def test_invalid_model_cache_does_not_start_synthesis(self):
        with patch.object(service.importlib.util, "find_spec", return_value=object()), patch.object(
            service, "_local_model_info", return_value=(False, None)
        ), patch.object(service, "VieNeuTTSProvider") as provider:
            with self.assertRaisesRegex(RuntimeError, "Model VieNeu-TTS chưa sẵn sàng"):
                service.synthesize({"cacheDir": "unused", "device": "cpu"}, lambda _event: None)
        provider.assert_not_called()


if __name__ == "__main__":
    unittest.main()
