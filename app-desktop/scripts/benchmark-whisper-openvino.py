"""Explicit local benchmark; never downloads a runtime, model, or audio."""
import argparse
import json
import logging
import pathlib
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / 'python'))
from worker.asr.openvino_backend import OpenVinoWhisperBackend, wav_duration

parser = argparse.ArgumentParser()
parser.add_argument('--model-dir', required=True)
parser.add_argument('--audio', required=True)
parser.add_argument('--language', default='auto')
parser.add_argument('--cache-dir')
args = parser.parse_args()
logging.basicConfig(level=logging.INFO)
audio = pathlib.Path(args.audio)
backend = OpenVinoWhisperBackend(pathlib.Path(args.model_dir), pathlib.Path(args.cache_dir) if args.cache_dir else None)
started = time.monotonic()
backend.load()
loaded = time.monotonic()
segments, language = backend.transcribe(audio, None if args.language == 'auto' else args.language, lambda value: print(f'progress={value:.3f}', file=sys.stderr))
elapsed = time.monotonic() - loaded
print(json.dumps({'device': backend.device, 'loadSeconds': loaded-started, 'elapsedSeconds': elapsed, 'duration': wav_duration(audio), 'rtf': elapsed/wav_duration(audio), 'language': language, 'segments': segments}, ensure_ascii=False, indent=2))
backend.close()
