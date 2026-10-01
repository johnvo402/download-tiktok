"""OpenVINO execution adapter for RapidOCR 3.9.2's public processing classes.

Only constructors are supplied here; detection, crop, orientation, CTC decoding
and output assembly use the same RapidOCR implementations as ORT. No package
files or global factories are changed. Models are compiled once per engine.
RapidOCR/Paddle processing components are Apache-2.0 licensed.
"""
from __future__ import annotations

import io
import logging
from pathlib import Path
from threading import Lock

import numpy as np
from rapidocr import RapidOCR
from rapidocr.cal_rec_boxes import CalRecBoxes
from rapidocr.ch_ppocr_cls import TextClassifier
from rapidocr.ch_ppocr_cls.main import CLS_SHAPE_BY_OCR_VERSION
from rapidocr.ch_ppocr_cls.utils import ClsPostProcess
from rapidocr.ch_ppocr_det import TextDetector
from rapidocr.ch_ppocr_det.utils import DBPostProcess
from rapidocr.ch_ppocr_rec import TextRecognizer
from rapidocr.ch_ppocr_rec.utils import CTCLabelDecode
from rapidocr.utils.load_image import LoadImage
from rapidocr.utils.parse_parameters import ParseParams

LOGGER = logging.getLogger(__name__)


def _varint(stream) -> int:
    result = 0
    for shift in range(0, 70, 7):
        value = stream.read(1)
        if not value:
            raise ValueError("Incomplete ONNX metadata")
        result |= (value[0] & 127) << shift
        if value[0] < 128:
            return result
    raise ValueError("Invalid ONNX varint")


def _string_fields(stream):
    # Stream-skip ONNX weight tensors; only materialize small metadata entries.
    while stream.read(1):
        stream.seek(-1, 1)
        tag = _varint(stream)
        wire = tag & 7
        if wire == 0:
            _varint(stream)
        elif wire in (1, 5):
            stream.seek(8 if wire == 1 else 4, 1)
        elif wire == 2:
            length = _varint(stream)
            yield tag >> 3, length, stream
            stream.seek(length, 1)
        else:
            raise ValueError("Unsupported ONNX metadata wire type")


def model_characters(path: Path) -> list[str]:
    """Read ONNX's character dictionary without importing onnx or creating a CPU session."""
    with path.open("rb") as stream:
        for field, length, value in _string_fields(stream):
            if field != 14:
                continue
            if length > 8 * 1024 * 1024:
                raise ValueError("ONNX metadata too large")
            position = value.tell()
            entry = io.BytesIO(value.read(length))
            value.seek(position)
            strings = {}
            for item, size, data in _string_fields(entry):
                before = data.tell()
                strings[item] = data.read(size).decode("utf-8")
                data.seek(before)
            if strings.get(1) == "character":
                characters = strings[2].splitlines()
                if not characters:
                    raise ValueError("Bảng ký tự trong model OCR bị rỗng.")
                return characters
    raise ValueError("Không tìm thấy bảng ký tự trong model OCR. Hãy cài lại model đang chọn.")


class OpenVinoSession:
    def __init__(self, core, model_path: Path, device: str):
        model = core.read_model(str(model_path))
        self.compiled = core.compile_model(model, device, {"PERFORMANCE_HINT": "LATENCY"})
        self.devices = list(self.compiled.get_property("EXECUTION_DEVICES"))
        if not self.devices or any(not value.startswith("GPU") for value in self.devices):
            raise RuntimeError("Model OCR không chạy trên GPU Intel; đã dừng để tránh dùng CPU ngoài ý muốn.")
        self.session = self.compiled.create_infer_request()
        self.lock = Lock()
        LOGGER.info("RapidOCR OpenVINO compiled: model=%s actual=%s", model_path.name, self.devices)

    def __call__(self, image: np.ndarray) -> np.ndarray:
        with self.lock:
            self.session.infer({0: np.ascontiguousarray(image, dtype=np.float32)})
            # RapidOCR may retain outputs during subsequent inferences.
            return self.session.get_output_tensor(0).data.copy()


class OpenVinoDetector(TextDetector):
    def __init__(self, cfg, session):
        self.limit_side_len, self.limit_type = cfg.limit_side_len, cfg.limit_type
        self.mean, self.std = cfg.mean, cfg.std
        self.preprocess_op = None
        keys = ("thresh", "box_thresh", "max_candidates", "unclip_ratio", "use_dilation", "score_mode")
        self.postprocess_op = DBPostProcess(**{key: cfg[key] for key in keys})
        self.session = session


class OpenVinoClassifier(TextClassifier):
    def __init__(self, cfg, session):
        self.cls_image_shape = CLS_SHAPE_BY_OCR_VERSION[cfg.ocr_version]
        self.cls_batch_num, self.cls_thresh = cfg.cls_batch_num, cfg.cls_thresh
        self.postprocess_op = ClsPostProcess(cfg.label_list)
        self.session = session


class OpenVinoRecognizer(TextRecognizer):
    def __init__(self, cfg, session, characters):
        self.session = session
        self.postprocess_op = CTCLabelDecode(character=characters, character_path=None)
        self.rec_batch_num, self.rec_image_shape = cfg.rec_batch_num, cfg.rec_img_shape
        self.cfg = cfg
        self.RTL_LANGS = {"arabic", "fa", "ur"}


class RapidOCROpenVinoBackend(RapidOCR):
    def __init__(self, params: dict, model_files: dict[str, Path], required_files: tuple[str, ...], device: str, runtime_cache: Path | None = None):
        import openvino as ov
        import rapidocr
        cfg = ParseParams.update_batch(ParseParams.load(Path(rapidocr.__file__).parent / "config.yaml"), params)
        core = ov.Core()
        if runtime_cache is not None:
            try:
                runtime_cache.mkdir(parents=True, exist_ok=True)
                if runtime_cache.is_symlink():
                    raise ValueError("OpenVINO cache is a symlink")
                core.set_property({"CACHE_DIR": str(runtime_cache)})
            except Exception:
                LOGGER.warning("OpenVINO compiled cache unavailable; compiling in memory", exc_info=True)
        det_path, rec_path, cls_path = (model_files[name] for name in required_files)
        # All three sessions must be Intel GPU; never call RapidOCR's CPU-only
        # OpenVINO constructor or create a CUDA/ORT session as an intermediary.
        self.text_det = OpenVinoDetector(cfg.Det, OpenVinoSession(core, det_path, device))
        self.text_cls = OpenVinoClassifier(cfg.Cls, OpenVinoSession(core, cls_path, device))
        cfg.Rec.font_path = cfg.Global.font_path
        self.text_rec = OpenVinoRecognizer(cfg.Rec, OpenVinoSession(core, rec_path, device), model_characters(rec_path))
        self.text_score, self.min_height = cfg.Global.text_score, cfg.Global.min_height
        self.width_height_ratio = cfg.Global.width_height_ratio
        self.use_det, self.use_cls, self.use_rec = cfg.Global.use_det, cfg.Global.use_cls, cfg.Global.use_rec
        self.load_img = LoadImage()
        self.max_side_len, self.min_side_len = cfg.Global.max_side_len, cfg.Global.min_side_len
        self.cal_rec_boxes = CalRecBoxes()
        self.return_word_box, self.return_single_char_box = cfg.Global.return_word_box, cfg.Global.return_single_char_box
        self.cfg = cfg
        self.lock = Lock()
        self.backend = "openvino"
        self.device = device

    def __call__(self, image, **kwargs):
        # The inherited pipeline updates preprocessing parameters per call.
        with self.lock:
            try:
                return super().__call__(image, **kwargs)
            except Exception as error:
                LOGGER.exception("OpenVINO OCR inference failed: device=%s", self.device)
                raise RuntimeError("Không thể đọc chữ bằng GPU Intel. Kiểm tra driver Intel hoặc chọn Tự động/CPU.") from error
