"""Explicit dynamic packages and licenses, without tests/examples/model weights."""
import sys
from pathlib import Path
from importlib.metadata import packages_distributions, distribution
from PyInstaller.utils.hooks import collect_all, collect_data_files, copy_metadata

def production_module(name):
    return not any(part in {'tests', 'test', 'testing', 'examples', 'demos', 'conftest'} for part in name.split('.'))

def dynamic_package(name):
    data, binaries, hidden = collect_all(name, filter_submodules=production_module)
    data = [pair for pair in data if not any(part in {'tests', 'test', 'testing', 'examples', 'demos', '__pycache__'}
             for part in Path(pair[0]).parts)]
    if name == 'rapidocr':
        data = [pair for pair in data if 'models' not in Path(pair[0]).parts]
    return data, binaries, hidden

def attach_licenses(analysis):
    # Inventory reflects modules actually found by Analysis, rather than every
    # distribution installed in the build venv (which also contains build tools).
    mapping = packages_distributions()
    roots = {row[0].split('.')[0] for row in analysis.pure}
    roots |= {row[0].split('/')[0].split('\\')[0].split('.')[0] for row in analysis.binaries}
    used = {name for module in roots for name in mapping.get(module, [])}
    # Native packages can use a binary basename unrelated to the module.
    for name in ('numpy', 'onnxruntime', 'ctranslate2', 'opencv-python-headless', 'av', 'curl-cffi'):
        try:
            dist = distribution(name)
        except Exception:
            continue
        if any(module in roots for module in {'numpy', 'onnxruntime', 'ctranslate2', 'cv2', 'av', 'curl_cffi'}
               if name == {'numpy':'numpy','onnxruntime':'onnxruntime','ctranslate2':'ctranslate2','cv2':'opencv-python-headless','av':'av','curl_cffi':'curl-cffi'}[module]):
            used.add(dist.metadata['Name'])
    # PyInstaller's bootloader is shipped, even though the build tool is not.
    used.add('pyinstaller')
    python_license = Path(sys.base_prefix) / 'LICENSE.txt'
    if not python_license.is_file():
        raise FileNotFoundError('CPython license is required in every worker')
    analysis.datas += [('licenses/CPython-LICENSE.txt', str(python_license), 'DATA')]
    for name in sorted(used):
        for source, target in copy_metadata(name):
            path = Path(source)
            if path.is_dir():
                analysis.datas += [(str(Path(target)/p.relative_to(path)), str(p), 'DATA')
                    for p in path.rglob('*') if p.is_file()]
            else:
                analysis.datas += [(str(Path(target)/path.name), str(path), 'DATA')]
