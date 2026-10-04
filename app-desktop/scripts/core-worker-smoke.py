"""Opt-in real inference using already installed models; never downloads weights.

Outputs and temporary frames stay in .release-cache/task09-core-qa. The legacy
build venv is used as a comparison for ASR/OCR, not as a release dependency.
"""
from pathlib import Path
import json
import os
import shutil
import subprocess
import time
import wave

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = Path(os.environ.get('JC_TASK09_CORE_OUTPUT', str(ROOT / '.release-cache/task09-core-qa')))
BUNDLE = Path(os.environ.get('JC_TASK09_BUNDLE_ROOT', str(ROOT / 'src-tauri/resources')))
FROZEN_ONLY = os.environ.get('JC_TASK09_FROZEN_ONLY') == '1'
MODELS = Path(os.environ['APPDATA']) / 'com.ai.video-localization/models'


def run(command, request):
    began = time.perf_counter()
    environment = {**os.environ, 'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1',
                   'PYTHONUTF8': '1', 'JC_AI_GPU_RUNTIME': '0'}
    for key in ['PYTHONHOME', 'PYTHONUSERBASE', 'LOCALIZATION_PYTHON']:
        environment.pop(key, None)
    if FROZEN_ONLY:
        environment.pop('PYTHONPATH', None)
        environment['PATH'] = str(Path(os.environ['SystemRoot']) / 'System32')
    else:
        environment['PYTHONPATH'] = str(ROOT / 'python')
    result = subprocess.run(command, input=json.dumps(request, ensure_ascii=False) + '\n',
        encoding='utf-8', capture_output=True, timeout=300, cwd=ROOT / 'python',
        creationflags=subprocess.CREATE_NO_WINDOW,
        env=environment)
    messages = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    failures = [m for m in messages if m['type'] in {'failed', 'segment_failed'}]
    if result.returncode or failures or not any(m['type'] == 'completed' for m in messages):
        raise AssertionError(f'{request["operation"]}: {failures}; exit={result.returncode}; {result.stderr[-3000:]}')
    return {'seconds': time.perf_counter()-began, 'messages': len(messages)}


def main():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    frames = OUTPUT / 'frames'
    frames.mkdir(exist_ok=True)
    fixture = ROOT / '.release-cache/sttn-qa/quality-matrix/two-line-Chinese/frames/frame_00000001.jpg'
    shutil.copyfile(fixture, frames / 'frame_00000001.jpg')
    audio = ROOT / '.release-cache/whisper-openvino-validation/speech.wav'
    variants = {
        'legacy': [str(ROOT/'python/.release-venv/Scripts/python.exe'), '-m', 'worker'],
        'frozen': [str(BUNDLE/'ai-worker/ai-worker.exe')],
    }
    if FROZEN_ONLY:
        variants.pop('legacy')
    report = {}
    for variant, command in variants.items():
        asr_path, ocr_path, voice_path = (OUTPUT/f'{variant}-{kind}.{extension}' for kind, extension in
            [('asr','json'),('ocr','json'),('tts','wav')])
        result = {}
        result['asr'] = run(command, {'operation':'transcribe','engine':'fasterWhisper','model':'medium',
            'device':'cpu','language':'vi','audioPath':str(audio),'outputPath':str(asr_path),'cacheDir':str(MODELS/'fw')})
        result['ocr'] = run(command, {'operation':'ocr_transcribe','model':'tiny','device':'cpu','fps':1,
            'framesDir':str(frames),'outputPath':str(ocr_path),'cacheDir':str(MODELS)})
        result['tts'] = run(command, {'operation':'tts_generate','device':'cpu','cacheDir':str(MODELS/'huggingface'),
            'segments':[{'id':'qa','text':'Xin chào. Đây là bản kiểm tra giọng đọc tiếng Việt.',
                         'voice':'Ngọc Huyền','outputPath':str(voice_path)}]})
        with wave.open(str(voice_path)) as wav:
            result['tts'].update(sampleRate=wav.getframerate(), duration=wav.getnframes()/wav.getframerate())
            assert wav.getnframes()>0 and wav.getframerate()>0
        result['asr']['output'] = json.loads(asr_path.read_text(encoding='utf-8'))
        result['ocr']['output'] = json.loads(ocr_path.read_text(encoding='utf-8'))
        report[variant] = result
        (OUTPUT/'results.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
        print(json.dumps({variant:{k:{a:b for a,b in v.items() if a!='output'} for k,v in result.items()}},ensure_ascii=True),flush=True)
    def content(value):
        # IDs are newly generated for each transcript; timing/text must match.
        items = value if isinstance(value,list) else value['segments']
        return [(s['start'],s['end'],s.get('source',s.get('text'))) for s in items]
    for kind in ([] if FROZEN_ONLY else ['asr','ocr']):
        assert content(report['legacy'][kind]['output']) == content(report['frozen'][kind]['output']), kind
    print('Frozen CPU ASR/OCR/TTS inference passed without system Python.' if FROZEN_ONLY else
          'Real ASR/OCR timing and text match; both CPU TTS outputs contain valid PCM.', flush=True)


if __name__=='__main__':
    main()
