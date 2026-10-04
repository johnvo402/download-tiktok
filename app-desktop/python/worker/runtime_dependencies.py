"""Dependency reachability for optional-pack uninstall; imports no model SDKs."""
from __future__ import annotations
from importlib.metadata import distributions
from packaging.markers import default_environment
from packaging.requirements import Requirement
from packaging.utils import canonicalize_name

def unused_packages(roots: list[str], installed: dict[str, list[str]], cuda_ort: bool = False) -> list[str]:
    graph = {canonicalize_name(name): deps for name, deps in installed.items()}
    pending = [(canonicalize_name(name), set()) for name in roots]
    pending.append(('packaging', set()))
    keep: dict[str, set[str]] = {}
    environment = default_environment()
    def substitute(name):
        if name == 'onnxruntime' and cuda_ort: return 'onnxruntime-gpu'
        if name == 'opencv-python' and name not in graph and 'opencv-python-headless' in graph:
            return 'opencv-python-headless'
        return name
    while pending:
        name, extras = pending.pop()
        name = substitute(name)
        if name not in graph:
            continue
        if name in keep and extras <= keep[name]:
            continue
        keep.setdefault(name, set()).update(extras)
        for raw in graph[name]:
            requirement = Requirement(raw)
            if requirement.marker and not any(requirement.marker.evaluate({**environment, 'extra': extra}) for extra in {'', *keep[name]}):
                continue
            pending.append((canonicalize_name(requirement.name), set(requirement.extras)))
    return sorted(set(graph) - set(keep))

if __name__ == '__main__':
    import json, sys
    request = json.loads(sys.argv[1])
    installed = {d.metadata['Name']: list(d.requires or []) for d in distributions()}
    if request.get('inventory'):
        packages = []
        for dist in distributions():
            license_files = [str(file).replace('\\', '/') for file in dist.files or []
                if 'license' in str(file).lower() or 'copying' in str(file).lower()]
            packages.append({'name': dist.metadata['Name'], 'version': dist.version,
                'license': dist.metadata.get('License-Expression') or dist.metadata.get('License') or 'See license files',
                'licenseFiles': license_files})
        print(json.dumps({'scope': 'installed optional runtime packages', 'packages': sorted(packages, key=lambda p:p['name'].lower())}))
    else:
        print(json.dumps(unused_packages(request['roots'], installed, request.get('cudaOrt', False))))
