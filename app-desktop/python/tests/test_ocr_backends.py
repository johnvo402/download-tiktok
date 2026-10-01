import os
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from worker.ocr.backends import OcrRuntimeCapability, probe_openvino_gpu, resolve_ocr_backend
from worker.ocr.service import _extract_text, verify_engine_device


def capabilities(cuda=False, intel=False):
    return {key: OcrRuntimeCapability(key, ready, device, key, message="Unavailable")
            for key, ready, device in (("cpu", True, "CPU"), ("cuda", cuda, "CUDA"), ("openvino", intel, "GPU.1"))}


class BackendResolverTests(unittest.TestCase):
    def test_auto_prefers_cuda_over_intel(self):
        self.assertEqual(resolve_ocr_backend("auto", capabilities(True, True)).backend, "cuda")

    def test_auto_selects_intel_after_unavailable_cuda(self):
        self.assertEqual(resolve_ocr_backend("auto", capabilities(False, True)).backend, "openvino")

    def test_auto_cpu_fallback(self):
        self.assertEqual(resolve_ocr_backend("auto", capabilities()).backend, "cpu")

    def test_priority_is_configurable(self):
        self.assertEqual(resolve_ocr_backend("auto", capabilities(True, True), ("openvino", "cuda", "cpu")).backend, "openvino")

    def test_manual_gpu_does_not_fallback(self):
        for backend in ("cuda", "openvino"):
            with self.subTest(backend=backend), self.assertRaisesRegex(RuntimeError, "Unavailable"):
                resolve_ocr_backend(backend, capabilities())

    def test_cpu_ignores_ready_gpus(self):
        self.assertEqual(resolve_ocr_backend("cpu", capabilities(True, True)).backend, "cpu")

    def test_invalid_backend_is_rejected(self):
        with self.assertRaises(ValueError):
            resolve_ocr_backend("invented", capabilities())

    def test_cpu_selection_does_not_probe_gpus(self):
        with patch("worker.ocr.backends.cpu_capability", return_value=capabilities()["cpu"]), patch("worker.ocr.backends.runtime_capabilities") as probe:
            resolve_ocr_backend("cpu")
            probe.assert_not_called()


class OpenVinoProbeTests(unittest.TestCase):
    def setUp(self):
        try:
            import numpy as np
        except ImportError:
            self.skipTest("numpy not installed in this test interpreter")
        self.np = np

    def core(self, devices=("GPU.1", "CPU"), actual=("GPU.1",), output=3.0):
        compiled = Mock()
        compiled.get_property.return_value = list(actual)
        compiled.return_value = {"sum": self.np.array([output], dtype=self.np.float32)}
        core = Mock(available_devices=list(devices))
        core.compile_model.return_value = compiled
        return core, compiled

    def test_compiles_runs_and_verifies_actual_gpu(self):
        core, compiled = self.core()
        cap = probe_openvino_gpu(lambda: core)
        self.assertTrue(cap.available)
        self.assertEqual(cap.device, "GPU.1")
        core.read_model.assert_called_once()
        self.assertIsInstance(core.read_model.call_args.args[0], bytes)
        self.assertEqual(core.compile_model.call_args.args[1], "GPU.1")
        compiled.assert_called_once()

    def test_device_listing_without_inference_is_not_ready(self):
        core, compiled = self.core()
        compiled.side_effect = RuntimeError("driver broken")
        self.assertFalse(probe_openvino_gpu(lambda: core).available)

    def test_cpu_execution_is_rejected(self):
        core, _ = self.core(actual=("CPU",))
        self.assertFalse(probe_openvino_gpu(lambda: core).available)

    def test_wrong_inference_output_is_rejected(self):
        core, _ = self.core(output=4.0)
        self.assertFalse(probe_openvino_gpu(lambda: core).available)

    def test_missing_gpu_is_graceful(self):
        core, _ = self.core(devices=("CPU",))
        self.assertFalse(probe_openvino_gpu(lambda: core).available)
        core.compile_model.assert_not_called()

    def test_multiple_devices_are_deterministic_and_try_next_failure(self):
        core, compiled = self.core(devices=("GPU.2", "GPU.1", "CPU"), actual=("GPU.2",))
        core.compile_model.side_effect = [RuntimeError("GPU.1 unusable"), compiled]
        cap = probe_openvino_gpu(lambda: core)
        self.assertEqual(cap.device, "GPU.2")
        self.assertTrue(cap.available)


class EngineContractTests(unittest.TestCase):
    def test_unicode_text_contract_is_same_for_all_backends(self):
        boxes = [[[0, 0], [100, 0], [100, 20], [0, 20]]]
        for backend in ("cpu", "cuda", "openvino"):
            with self.subTest(backend=backend):
                result = SimpleNamespace(boxes=boxes, txts=("Tiếng Việt 中文",), scores=(0.95,))
                self.assertEqual(_extract_text(result), "Tiếng Việt 中文")

    def test_openvino_verifies_all_component_devices(self):
        engine = SimpleNamespace(backend="openvino")
        for name in ("text_det", "text_cls", "text_rec"):
            setattr(engine, name, SimpleNamespace(session=SimpleNamespace(devices=["GPU.1"])))
        self.assertEqual(len(verify_engine_device(engine, "openvino")), 3)
        engine.text_rec.session.devices = ["CPU"]
        with self.assertRaises(RuntimeError):
            verify_engine_device(engine, "openvino")

    def test_manual_openvino_cannot_accept_ort_engine(self):
        with self.assertRaises(RuntimeError):
            verify_engine_device(SimpleNamespace(), "openvino")

    def test_explicit_auto_model_fallback_is_allowed(self):
        self.assertEqual(verify_engine_device(SimpleNamespace(ocr_auto_fallback=True), "openvino"), {})


class AutoEngineTests(unittest.TestCase):
    def test_model_inference_failure_tries_next_backend_once_then_reuses_cpu(self):
        from worker.ocr.service import _AutoOcrEngine
        original = Mock(side_effect=RuntimeError("GPU operator failed"))
        cpu = Mock(return_value="normalized result")
        fallback = Mock(side_effect=[RuntimeError("Intel unavailable"), cpu])
        engine = _AutoOcrEngine(original, "cuda", fallback)
        with patch("worker.ocr.service.verify_engine_device") as verify:
            self.assertEqual(engine("frame1"), "normalized result")
            self.assertEqual(engine("frame2"), "normalized result")
            verify.assert_called_once()
        self.assertEqual([call.args[0] for call in fallback.call_args_list], ["openvino", "cpu"])
        self.assertEqual(engine.resolved_backend, "cpu")
        self.assertTrue(engine.ocr_auto_fallback)

    def test_manual_selection_is_never_wrapped(self):
        from worker.ocr.service import _auto_wrap
        engine = Mock()
        for backend in ("cuda", "openvino", "cpu"):
            self.assertIs(_auto_wrap(engine, backend, backend, "models", "tiny"), engine)

    def test_healthy_auto_engine_is_reused_without_recompiling(self):
        from worker.ocr.service import _AutoOcrEngine
        original = Mock(return_value="normalized result")
        fallback = Mock()
        engine = _AutoOcrEngine(original, "openvino", fallback)
        with patch("worker.ocr.service.verify_engine_device") as verify:
            engine("frame1")
            engine("frame2")
            verify.assert_called_once()
        fallback.assert_not_called()


class OpenVinoAdapterTests(unittest.TestCase):
    def setUp(self):
        try:
            from worker.ocr.openvino_engine import OpenVinoSession, model_characters
            import numpy as np
        except ImportError:
            self.skipTest("RapidOCR/numpy not installed in this interpreter")
        self.session = OpenVinoSession
        self.characters = model_characters
        self.np = np

    def test_session_compiles_once_and_copies_output(self):
        core = Mock()
        compiled = core.compile_model.return_value
        compiled.get_property.return_value = ["GPU.0"]
        data = self.np.array([1.0], dtype=self.np.float32)
        request = compiled.create_infer_request.return_value
        request.get_output_tensor.return_value = SimpleNamespace(data=data)
        session = self.session(core, Path("shared.onnx"), "GPU")
        first = session(self.np.array([0.0]))
        data[0] = 2.0
        second = session(self.np.array([0.0]))
        self.assertEqual(first.tolist(), [1.0])
        self.assertEqual(second.tolist(), [2.0])
        core.compile_model.assert_called_once()
        self.assertEqual(request.infer.call_count, 2)

    def test_session_refuses_cpu_execution(self):
        core = Mock()
        core.compile_model.return_value.get_property.return_value = ["CPU"]
        with self.assertRaises(RuntimeError):
            self.session(core, Path("shared.onnx"), "GPU")

    def test_shared_onnx_dictionary_preserves_unicode(self):
        from worker.ocr.service import _protobuf_bytes
        entry = _protobuf_bytes(1, b"character") + _protobuf_bytes(2, "你\nViệt\n字".encode("utf-8"))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "shared.onnx"
            path.write_bytes(_protobuf_bytes(7, b"x" * 1024) + _protobuf_bytes(14, entry))
            self.assertEqual(self.characters(path), ["你", "Việt", "字"])


@unittest.skipUnless(os.environ.get("JC_AI_TEST_OPENVINO") == "1", "Intel inference test is opt-in")
class IntelHardwareTests(unittest.TestCase):
    def test_real_add_on_gpu(self):
        cap = probe_openvino_gpu()
        if not cap.available:
            self.skipTest(cap.message)
        self.assertTrue(cap.device.startswith("GPU"))


if __name__ == "__main__":
    unittest.main()
