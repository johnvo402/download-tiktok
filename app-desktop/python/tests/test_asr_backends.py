import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch
import wave

from worker.asr.backends import resolve_asr_backend, normalize_segments
from worker.asr.openvino_backend import OpenVinoWhisperBackend
from worker.asr.openvino_models import artifact, model_directory, model_ready, ARTIFACTS, MANIFEST
from worker.model_manifest import create_manifest


class SelectionTests(unittest.TestCase):
    def test_auto_priority(self):
        for capabilities, expected in [({'cpu': True, 'cuda': True, 'openvino': True}, 'cuda'),
                                        ({'cpu': True, 'openvino': True}, 'openvino'), ({'cpu': True}, 'cpu')]:
            result = resolve_asr_backend('auto', 'auto', capabilities)
            self.assertEqual(result.device, expected)
            self.assertNotEqual(result.engine, 'whisperX')

    def test_manual_and_engine_constraints(self):
        self.assertEqual(resolve_asr_backend('auto', 'cpu', {'cpu': True, 'openvino': True}).device, 'cpu')
        self.assertEqual(resolve_asr_backend('fasterWhisper', 'auto', {'cpu': True, 'openvino': True}).device, 'cpu')
        for device in ['cuda', 'openvino']:
            with self.assertRaises(RuntimeError):
                resolve_asr_backend('auto', device, {'cpu': True})
        for engine, device in [('whisperX', 'openvino'), ('fasterWhisper', 'openvino'), ('openVinoWhisper', 'cpu')]:
            with self.assertRaises(ValueError):
                resolve_asr_backend(engine, device, {'cpu': True, 'openvino': True})

    def test_timestamps(self):
        result = normalize_segments([{'start': 3, 'end': 4, 'text': '  Việt\nNam '}, {'start': 0, 'end': 1, 'text': '中'}], 5)
        self.assertEqual(result[0]['text'], '中')
        self.assertEqual(result[1]['text'], 'Việt\nNam')
        for start, end in [(float('nan'), 1), (0, float('inf')), (-1, 1), (3, 3), (4, 6)]:
            with self.assertRaises(ValueError):
                normalize_segments([{'start': start, 'end': end, 'text': 'test'}], 5)
        self.assertEqual(normalize_segments([{'start': 0, 'end': 1, 'text': ' '}], 5), [])


class ModelTests(unittest.TestCase):
    def test_missing_ct2_only_and_incomplete(self):
        with tempfile.TemporaryDirectory() as folder:
            cache = Path(folder)/'models/fw'
            (cache/'tiny').mkdir(parents=True)
            (cache/'tiny/model.bin').write_bytes(b'CT2')
            self.assertFalse(model_ready(cache, 'tiny'))
            root = model_directory(cache, 'tiny')
            root.mkdir(parents=True)
            (root/'openvino_encoder_model.xml').write_text('partial')
            self.assertFalse(model_ready(cache, 'tiny'))
            self.assertTrue((cache/'tiny/model.bin').exists())
        with self.assertRaises(ValueError): artifact('large-v3')
        self.assertFalse(model_ready('unused', 'medium'))

    def test_manifest_revision_verification_and_integrity(self):
        with tempfile.TemporaryDirectory() as folder:
            cache=Path(folder)/'fw'; root=model_directory(cache, 'tiny'); root.mkdir(parents=True)
            info={'repository':'test/model', 'revision':'pinned', 'files':[{'name':'weights.bin','size':4}]}
            (root/'weights.bin').write_bytes(b'good')
            (root/'gpu-verified.json').write_text(json.dumps({'schemaVersion':1,'revision':'pinned','device':'GPU.0'}))
            create_manifest(root,root/MANIFEST,'test/model',[root/'weights.bin', root/'gpu-verified.json'],model_id='tiny',version='pinned',download='https://example.com')
            with patch.dict(ARTIFACTS, {'tiny':info}):
                self.assertTrue(model_ready(cache,'tiny'))
                (root/'weights.bin').write_bytes(b'fail!')
                self.assertFalse(model_ready(cache,'tiny'))


class ChunkTests(unittest.TestCase):
    def audio(self, root, seconds):
        path=Path(root)/'audio.wav'
        with wave.open(str(path),'wb') as output:
            output.setparams((1,2,16000,0,'NONE','not compressed'))
            output.writeframes(b'\0\0' * (seconds*16000))
        return path

    def test_lifecycle_offsets_overlap_language_and_progress(self):
        calls=[]
        chunks=[[SimpleNamespace(start_ts=0,end_ts=1,text='你好'),SimpleNamespace(start_ts=28,end_ts=30,text='boundary')],
                [SimpleNamespace(start_ts=0,end_ts=2,text='boundary'),SimpleNamespace(start_ts=3,end_ts=4,text='日本語')]]
        class Pipeline:
            def generate(self,pcm,**kwargs):
                calls.append((len(pcm),kwargs)); return SimpleNamespace(chunks=[chunks[len(calls)-1]],languages=['zh'])
        factory_calls=[]
        def factory(path,device,**kwargs): factory_calls.append(device);return Pipeline()
        with tempfile.TemporaryDirectory() as folder, patch('worker.ocr.backends.probe_openvino_gpu',return_value=SimpleNamespace(available=True,device='GPU.0')):
            backend=OpenVinoWhisperBackend(Path(folder),pipeline_factory=factory,pcm_decoder=lambda frames:range(len(frames)//2));backend.load()
            progress=[]; result,language=backend.transcribe(self.audio(folder,35),None,progress.append)
            self.assertEqual(factory_calls,['GPU.0'])
            self.assertEqual([item['text'] for item in result],['你好','boundary','日本語'])
            self.assertEqual(result[1]['start'],28)
            self.assertEqual(result[2]['start'],31)
            self.assertEqual(language,'zh')
            self.assertEqual(calls[1][1]['language'],'<|zh|>')
            self.assertLessEqual(max(length for length,_ in calls),30*16000)
            self.assertEqual(len(progress),2)
            backend.close();self.assertIsNone(backend.pipeline)

    def test_cancel_and_missing_end(self):
        with tempfile.TemporaryDirectory() as folder:
            backend=OpenVinoWhisperBackend(Path(folder),pcm_decoder=lambda frames:range(len(frames)//2))
            path=self.audio(folder,1)
            with self.assertRaises(InterruptedError): backend.transcribe(path,None,lambda _:None,lambda:True)
            backend.pipeline=SimpleNamespace(generate=lambda *_args,**_kwargs:SimpleNamespace(chunks=[[SimpleNamespace(start_ts=0,end_ts=-1,text='unfinished')]],languages=['en']))
            with self.assertRaises(RuntimeError):backend.transcribe(path,None,lambda _:None)

    def test_native_failure_is_logged_without_exposing_raw_exception(self):
        with tempfile.TemporaryDirectory() as folder:
            backend = OpenVinoWhisperBackend(Path(folder), pcm_decoder=lambda frames: range(len(frames) // 2))
            def fail(*args, **kwargs):
                raise RuntimeError('INTERNAL_GPU_EXCEPTION_SECRET_DETAILS')
            backend.pipeline = SimpleNamespace(generate=fail)
            with self.assertLogs('worker.asr.openvino_backend', level='ERROR') as logs:
                with self.assertRaises(RuntimeError) as caught:
                    backend.transcribe(self.audio(folder, 1), None, lambda _: None)
            self.assertNotIn('INTERNAL_GPU_EXCEPTION', str(caught.exception))
            self.assertIn('INTERNAL_GPU_EXCEPTION', '\n'.join(logs.output))


class OrchestrationTests(unittest.TestCase):
    def test_auto_failure_notifies_then_cpu_and_atomic_output(self):
        from worker.asr.service import transcribe
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder); audio=ChunkTests().audio(root,2); events=[]
            class Intel:
                def __init__(self,*a): pass
                def load(self):raise RuntimeError('model GPU failure')
                def close(self):pass
            class Cpu:
                model=None
                def __init__(self,*a):pass
                def load(self):pass
                def transcribe(self,*a):return [{'start':0,'end':1,'text':'hello'}],'en'
                def close(self):pass
            request={'audioPath':str(audio),'outputPath':str(root/'transcript.json'),'cacheDir':str(root/'fw'),'engine':'auto','device':'auto','model':'tiny'}
            with patch('worker.asr.backends.runtime_capabilities',return_value={'cpu':True,'openvino':True}),patch('worker.asr.openvino_models.model_ready',return_value=True),patch('worker.asr.service._model_is_complete',return_value=True),patch('worker.asr.openvino_backend.OpenVinoWhisperBackend',Intel),patch('worker.asr.backends.FasterWhisperBackend',Cpu):
                transcribe(request,events.append)
                self.assertTrue(any(item.get('stage')=='backend-fallback' and 'CPU' in item['message'] for item in events))
                self.assertEqual(events[-1]['type'],'completed')
                saved=(root/'transcript.json').read_bytes()
                request['device']='openvino';events.clear()
                with self.assertRaises(RuntimeError):transcribe(request,events.append)
                self.assertEqual((root/'transcript.json').read_bytes(),saved)
                self.assertFalse(any(item.get('stage')=='backend-fallback' for item in events))


if __name__=='__main__':unittest.main()
