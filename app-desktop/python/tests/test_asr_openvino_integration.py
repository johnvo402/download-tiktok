"""Opt-in real Intel inference. Never downloads anything in CI."""
import importlib.util
import os
from pathlib import Path
import time
import unittest

@unittest.skipUnless(os.environ.get('JC_AI_TEST_WHISPER_OPENVINO') == '1','Intel Whisper hardware test is opt-in')
class IntelWhisperTests(unittest.TestCase):
    def test_gpu_audio_timestamps_language_offline_without_torch(self):
        from worker.asr.openvino_backend import OpenVinoWhisperBackend,wav_duration
        from worker.ocr.backends import probe_openvino_gpu
        root=Path(os.environ.get('JC_AI_TEST_WHISPER_MODEL',''))
        audio=Path(os.environ.get('JC_AI_TEST_WHISPER_AUDIO',''))
        if not root.is_dir() or not audio.is_file() or not probe_openvino_gpu().available:
            self.skipTest('Installed IR, speech WAV or Intel GPU missing')
        self.assertIsNone(importlib.util.find_spec('torch'),'Run this test in the lightweight Intel environment')
        os.environ['HF_HUB_OFFLINE']='1'
        backend=OpenVinoWhisperBackend(root)
        try:
            backend.load();started=time.monotonic(); progress=[]
            segments,language=backend.transcribe(audio,None,progress.append)
            self.assertTrue(backend.device.startswith('GPU'))
            self.assertTrue(segments)
            self.assertEqual(language,'en')
            for item in segments:
                self.assertGreaterEqual(item['start'],0)
                self.assertGreater(item['end'],item['start'])
                self.assertLessEqual(item['end'],wav_duration(audio))
            print(f'Intel Whisper real RTF={(time.monotonic()-started)/wav_duration(audio):.3f}; segments={len(segments)}')
        finally:backend.close()
