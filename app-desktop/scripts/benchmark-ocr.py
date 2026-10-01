"""Opt-in real OCR benchmark. Uses installed models; never downloads weights.

Run with a Python containing rapidocr, onnxruntime, openvino:
python scripts/benchmark-ocr.py --models <app-data>/models --frames 12
Optionally supply --images <fixture-folder> for representative subtitle crops.
"""
import argparse
import json
import logging
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))
from worker.ocr.backends import probe_openvino_gpu
from worker.ocr.service import _create_engine, _extract_text, verify_engine_device


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--models", required=True)
    parser.add_argument("--model", choices=("tiny", "small", "medium"), default="tiny")
    parser.add_argument("--frames", type=int, default=12)
    parser.add_argument("--images", type=Path)
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO)
    images = sorted(args.images.glob("*.jpg")) if args.images else []
    if not images:
        from PIL import Image, ImageDraw, ImageFont
        import numpy as np
        sample = Image.new("RGB", (640, 160), "white")
        ImageDraw.Draw(sample).text((20, 40), "Hello world 12345", fill="black", font=ImageFont.truetype("arial.ttf", 48))
        images = [np.asarray(sample)]
    cap = probe_openvino_gpu()
    backends = ["cpu", "openvino"] if cap.available else ["cpu"]
    report = {"model": args.model, "openvino": cap.payload(), "results": []}
    for backend in backends:
        started = time.perf_counter()
        engine = _create_engine(args.models, args.model, backend)
        compile_seconds = time.perf_counter() - started
        warmup_started = time.perf_counter()
        engine(str(images[0]) if isinstance(images[0], Path) else images[0])
        warmup_seconds = time.perf_counter() - warmup_started
        sessions = verify_engine_device(engine, backend, "benchmark")
        started = time.perf_counter()
        texts = []
        for index in range(args.frames):
            image = images[index % len(images)]
            texts.append(_extract_text(engine(str(image) if isinstance(image, Path) else image)))
        elapsed = time.perf_counter() - started
        report["results"].append({"backend": backend, "devices": sessions, "frames": args.frames,
                                  "compileSeconds": compile_seconds, "warmupSeconds": warmup_seconds, "elapsedSeconds": elapsed,
                                  "msPerFrame": elapsed * 1000 / args.frames, "texts": texts})
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
