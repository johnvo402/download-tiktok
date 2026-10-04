from __future__ import annotations

import json
import sys
from email.parser import BytesParser
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: write_worker_inventory.py <PyInstaller worker directory>", file=sys.stderr)
        return 2
    root = Path(sys.argv[1]).resolve(strict=True)
    internal = root / "_internal"
    if not internal.is_dir():
        raise SystemExit("PyInstaller _internal directory is missing")

    packages: list[dict[str, object]] = []
    for metadata_path in sorted(internal.rglob("*.dist-info/METADATA")):
        metadata = BytesParser().parsebytes(metadata_path.read_bytes())
        dist_info = metadata_path.parent
        license_files: list[str] = []
        for candidate in sorted(dist_info.rglob("*")):
            if candidate.is_file() and candidate.name.casefold().startswith(("license", "copying", "notice")):
                try:
                    license_files.append(candidate.relative_to(root).as_posix())
                except ValueError:
                    continue
        direct_license = dist_info / "LICENSE"
        if direct_license.is_file() and direct_license.relative_to(root).as_posix() not in license_files:
            license_files.append(direct_license.relative_to(root).as_posix())
        packages.append({
            "name": metadata.get("Name", dist_info.name.removesuffix(".dist-info")),
            "version": metadata.get("Version", "unknown"),
            "license": metadata.get("License-Expression") or metadata.get("License") or "See included package metadata",
            "licenseFiles": license_files,
        })

    payload = {
        "schemaVersion": 1,
        "sourceLockFile": "python/requirements-downloader.txt" if root.name == "downloader-worker" else "python/requirements-release.txt + python/requirements-sdk.txt",
        "inventoryScope": "distributions used by PyInstaller Analysis plus the shipped bootloader; metadata and license files included",
        "packages": sorted(packages, key=lambda item: str(item["name"]).casefold()),
    }
    (root / "python-packages.json").write_text(
        json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps({"packageCount": len(packages), "inventory": "python-packages.json"}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
