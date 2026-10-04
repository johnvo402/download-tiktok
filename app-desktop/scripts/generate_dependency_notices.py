from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


LICENSE_NAMES = re.compile(r"^(license|copying|notice)([-_.].*)?$", re.IGNORECASE)
MAX_LICENSE_BYTES = 250_000


def _license_files(package_root: Path) -> list[Path]:
    candidates: list[Path] = []
    for child in package_root.iterdir():
        if child.is_file() and LICENSE_NAMES.match(child.name):
            candidates.append(child)
        elif child.is_dir() and child.name.casefold() in {"licenses", "license"}:
            candidates.extend(
                item for item in child.rglob("*")
                if item.is_file() and LICENSE_NAMES.match(item.name)
            )
    return sorted(candidates, key=lambda item: item.name.casefold())


def _read_license(path: Path) -> str | None:
    try:
        if path.stat().st_size > MAX_LICENSE_BYTES:
            return None
        return path.read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        return None


def _npm_packages(root: Path) -> list[dict[str, Any]]:
    lock = json.loads((root / "package-lock.json").read_text(encoding="utf-8"))
    packages: list[dict[str, Any]] = []
    for key, entry in lock.get("packages", {}).items():
        if not key.startswith("node_modules/") or entry.get("dev") is True:
            continue
        package_root = root / key
        package_json = package_root / "package.json"
        if not package_json.is_file():
            continue
        data = json.loads(package_json.read_text(encoding="utf-8"))
        licenses = _license_files(package_root)
        packages.append({
            "ecosystem": "npm",
            "name": data.get("name", package_root.name),
            "version": data.get("version", entry.get("version", "unknown")),
            "license": data.get("license", "See included package metadata"),
            "licenseFiles": licenses,
        })
    return packages


def _cargo_packages(root: Path) -> list[dict[str, Any]]:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--manifest-path", str(root / "src-tauri" / "Cargo.toml"), "--locked", "--filter-platform", "x86_64-pc-windows-msvc"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    metadata = json.loads(result.stdout)
    active_ids = {
        node.get("id")
        for node in (metadata.get("resolve") or {}).get("nodes", [])
        if isinstance(node.get("id"), str)
    }
    packages: list[dict[str, Any]] = []
    for package in metadata.get("packages", []):
        if not package.get("source") or package.get("id") not in active_ids:
            continue
        package_root = Path(package["manifest_path"]).parent
        explicit = package.get("license_file")
        licenses = [Path(explicit)] if explicit else _license_files(package_root)
        licenses = [path for path in licenses if path.is_file() and path.resolve().is_relative_to(package_root.resolve())]
        packages.append({
            "ecosystem": "cargo",
            "name": package["name"],
            "version": package["version"],
            "license": package.get("license") or "See included crate metadata",
            "licenseFiles": licenses,
        })
    return packages


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: generate_dependency_notices.py <repository root>", file=sys.stderr)
        return 2
    root = Path(sys.argv[1]).resolve(strict=True)
    output_dir = root / "src-tauri" / "resources"
    output_dir.mkdir(parents=True, exist_ok=True)
    packages = _npm_packages(root) + _cargo_packages(root)
    packages.sort(key=lambda item: (item["ecosystem"], item["name"].casefold(), item["version"]))

    notice_sections: list[str] = [
        "Third-party dependency license texts generated for this exact Windows build.",
        "Packages without a discovered license file retain their declared license metadata in dependency-license-inventory.json.",
    ]
    for package in packages:
        license_paths = package["licenseFiles"]
        package["licenseFiles"] = [path.name for path in license_paths]
        notice_sections.extend([
            "",
            "=" * 76,
            f"{package['ecosystem']}: {package['name']} {package['version']}",
            f"Declared license: {package['license']}",
            "=" * 76,
        ])
        found = False
        for path in license_paths:
            text = _read_license(path)
            if text:
                found = True
                notice_sections.extend([f"\n--- {path.name} ---\n", text])
        package["licenseTextIncluded"] = found

    inventory = {
        "schemaVersion": 1,
        "target": "windows-x64",
        "scope": "locked production npm packages and active x86_64-pc-windows-msvc Cargo dependencies",
        "packageCount": len(packages),
        "packages": packages,
    }
    (output_dir / "dependency-license-inventory.json").write_text(
        json.dumps(inventory, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    (output_dir / "THIRD_PARTY_DEPENDENCY_LICENSES.txt").write_text(
        "\n".join(line.rstrip() for line in "\n".join(notice_sections).splitlines()) + "\n", encoding="utf-8"
    )
    print(json.dumps({"packageCount": len(packages), "output": str(output_dir)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
