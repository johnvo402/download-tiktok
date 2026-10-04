"""Measure shipped artifacts and worker startup; never touches user projects/models."""
from __future__ import annotations
import argparse
import hashlib
import json
import statistics
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

def size_tree(path: Path) -> dict:
    files = [path] if path.is_file() else [p for p in path.rglob('*') if p.is_file() and not p.is_symlink()]
    dlls = [p for p in files if p.suffix.lower() in {'.dll', '.pyd'}]
    by_hash = {}
    for p in dlls:
        with p.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        by_hash.setdefault(digest, []).append(p)
    return {'bytes': sum(p.stat().st_size for p in files), 'files': len(files),
            'nativeFiles': len(dlls), 'top50': [{'path': str(p.relative_to(ROOT)), 'bytes': p.stat().st_size}
                for p in sorted(files, key=lambda p: p.stat().st_size, reverse=True)[:50]],
            'duplicateNativeFiles': [[str(p.relative_to(ROOT)) for p in group]
                for group in by_hash.values() if len(group) > 1]}

def startup(executable: Path) -> dict:
    # Repeated launches are file-cache warm measurements, NOT OS cold startup.
    import psutil
    samples = []
    for _ in range(3):
        began = time.perf_counter()
        p = subprocess.Popen([str(executable)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            creationflags=subprocess.CREATE_NO_WINDOW if hasattr(subprocess, 'CREATE_NO_WINDOW') else 0)
        p.stdin.close()
        peak = 0
        while p.poll() is None:
            try:
                process = psutil.Process(p.pid)
                peak = max(peak, sum(child.memory_info().rss for child in [process, *process.children(recursive=True)] if child.is_running()))
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                pass
            time.sleep(.01)
        samples.append({'seconds': time.perf_counter() - began, 'peakTreeRssBytes': peak, 'exitCode': p.returncode})
    return {'definition': 'Launch with closed stdin; no model or capability probe. OS file cache not flushed.',
            'samples': samples, 'medianSeconds': statistics.median(s['seconds'] for s in samples)}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    resources = ROOT / 'src-tauri/resources'
    report = {'measuredAt': datetime.now(timezone.utc).isoformat(), 'artifacts': {}}
    for name in ['ai-worker', 'downloader-worker', 'ffmpeg', 'optional-runtime']:
        path = resources / name
        if path.exists():
            report['artifacts'][name] = size_tree(path)
            inventory = path / 'python-packages.json'
            if inventory.exists():
                report['artifacts'][name]['packageInventory'] = json.loads(inventory.read_text(encoding='utf-8-sig'))
            exe = path / (name + '.exe')
            if name in {'ai-worker', 'downloader-worker'} and exe.exists():
                report['artifacts'][name]['startup'] = startup(exe)
    shared_native = {}
    for worker in ['ai-worker', 'downloader-worker']:
        for file in (resources / worker).rglob('*'):
            if file.is_file() and file.suffix.lower() in {'.dll', '.pyd'}:
                with file.open('rb') as stream:
                    digest = hashlib.file_digest(stream, 'sha256').hexdigest()
                shared_native.setdefault(digest, []).append(file)
    duplicates = [files for files in shared_native.values() if len(files) > 1
                  and len({str(p.relative_to(resources).parts[0]) for p in files}) > 1]
    report['crossWorkerNativeDuplicates'] = {
        'redundantBytes': sum(files[0].stat().st_size * (len(files)-1) for files in duplicates),
        'files': [[str(p.relative_to(ROOT)) for p in files] for files in duplicates],
        'note': 'Intentional independent frozen environments; not removed without ABI/loader verification.'}
    installer = ROOT / 'src-tauri/target/release/bundle/nsis/JC AI_0.1.13-preview_x64-setup.exe'
    if installer.exists():
        report['installer'] = {'path': str(installer), 'bytes': installer.stat().st_size,
            'modifiedAt': datetime.fromtimestamp(installer.stat().st_mtime, timezone.utc).isoformat(),
            'sha256': hashlib.sha256(installer.read_bytes()).hexdigest()}
    report['installedResourceBytes'] = sum(a['bytes'] for a in report['artifacts'].values())
    report['limitations'] = ['Installer may predate current source; timestamp/hash establish provenance.',
        'Resource bytes exclude app executable, WebView and user models; not total installed disk usage.',
        'No visible-window/first-project timing in this command.']
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({k: {'bytes': v['bytes'], 'files': v['files'], 'startup': v.get('startup')}
        for k, v in report['artifacts'].items()}, indent=2))

if __name__ == '__main__':
    main()
