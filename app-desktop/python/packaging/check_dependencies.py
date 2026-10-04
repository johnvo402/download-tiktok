"""Strict dependency check with two documented, tested SDK substitutions.

VieNeu's CPU ONNX path has a soundfile+soxr fallback for librosa and does not
import Gradio. RapidOCR uses cv2 without HighGUI; opencv-python-headless exports
the same cv2 API. No other missing/conflicting requirement is accepted.
"""
from importlib.metadata import distributions, distribution, PackageNotFoundError
from packaging.requirements import Requirement
from packaging.markers import default_environment
from packaging.utils import canonicalize_name

def check():
    errors = []
    environment = default_environment()
    environment['extra'] = ''
    for installed in distributions():
        owner = canonicalize_name(installed.metadata['Name'])
        for raw in installed.requires or []:
            requirement = Requirement(raw)
            if requirement.marker and not requirement.marker.evaluate(environment):
                continue
            name = canonicalize_name(requirement.name)
            if owner == 'vieneu' and name in {'gradio', 'librosa'}:
                continue
            if owner == 'rapidocr' and name == 'opencv-python':
                name = 'opencv-python-headless'
            try:
                actual = distribution(name).version
                if requirement.specifier and not requirement.specifier.contains(actual):
                    errors.append(f'{owner} requires {requirement}, found {actual}')
            except PackageNotFoundError:
                errors.append(f'{owner} requires missing {name}')
    if errors:
        raise SystemExit('\n'.join(errors))
    print('Verified dependencies (VieNeu web/demo extras omitted; RapidOCR cv2 is headless).')

if __name__ == '__main__':
    check()
