"""Opt-in extracted NSIS payload audit; no installer execution or user data writes."""
import argparse
import hashlib
import json
from pathlib import Path


def digest(file):
    with file.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('extracted', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    extracted = args.extracted.resolve()
    assert extracted.is_relative_to(root / '.release-cache'), 'Use an isolated extracted directory'
    manifest = json.loads((root / 'release/release-manifest.json').read_text(encoding='utf-8-sig'))
    artifact = root / 'release' / manifest['installer']
    assert artifact.name == f"JC-AI-{manifest['version']}-windows-x64-setup.exe"
    assert digest(artifact) == manifest['sha256']
    checks = []
    for group in ['ai-worker', 'downloader-worker', 'ffmpeg', 'optional-runtime']:
        source = root / 'src-tauri/resources' / group
        expected = {p.relative_to(source).as_posix(): p for p in source.rglob('*') if p.is_file()}
        actual = {p.relative_to(extracted / group).as_posix(): p for p in (extracted / group).rglob('*') if p.is_file()}
        assert set(expected) == set(actual), f'{group}: file inventory differs'
        for name, file in expected.items():
            assert digest(file) == digest(actual[name]), f'{group}: corrupt or mismatched {name}'
        checks.append({'group': group, 'files': len(actual), 'bytes': sum(p.stat().st_size for p in actual.values()), 'allHashesMatch': True})
    for name in ['THIRD_PARTY_LICENSES.txt', 'THIRD_PARTY_DEPENDENCY_LICENSES.txt', 'dependency-license-inventory.json']:
        assert digest(root / 'src-tauri/resources' / name) == digest(extracted / name)
    # Tauri patches a 13-byte bundle-type marker when packaging. It must be the
    # sole difference from the compiled exe, never a broad binary mask.
    executable = (root / 'src-tauri/target/release/ai-video-localization.exe').read_bytes()
    packaged = (extracted / 'ai-video-localization.exe').read_bytes()
    assert executable.replace(b'_TYPE_VAR_UNK', b'_TYPE_VAR_NSS') == packaged
    files = [p for p in extracted.rglob('*') if p.is_file() and '$PLUGINSDIR' not in p.relative_to(extracted).parts]
    previous = root / 'release/JC-AI-0.1.13-preview-windows-x64-setup.exe'
    report = {'version': manifest['version'], 'channel': manifest['channel'], 'sha256': manifest['sha256'],
              'signatureStatus': manifest['signatureStatus'], 'installerBytes': artifact.stat().st_size,
              'previousTask9InstallerBytes': previous.stat().st_size if previous.exists() else None,
              'extractedPayloadBytes': sum(p.stat().st_size for p in files), 'extractedPayloadFiles': len(files),
              'resources': checks, 'workerVersion': manifest['workerVersion'], 'workerProtocol': manifest['workerProtocolVersion'],
              'optionalRuntimeAbi': manifest['optionalRuntimeAbi'], 'passed': True,
              'limitations': ['Extraction/hash audit is not a clean-machine install, upgrade or uninstall test.',
                              'Payload size excludes separately installed WebView2, models, optional runtimes and user projects.']}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
