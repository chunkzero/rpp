#!/usr/bin/env python3
"""Add a packed plugin version to a checkout of the rpp registry."""

import argparse
import json
import re
import sys
from pathlib import Path

SEMVER_RE = re.compile(r"(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+.*)?")


def semver_key(version: str) -> tuple:
    m = SEMVER_RE.fullmatch(version)
    if not m:
        raise ValueError(f"invalid semver {version!r}")
    major, minor, patch, pre = m.groups()
    if pre is None:
        return (int(major), int(minor), int(patch), (1,))
    parts = tuple((0, int(p), "") if p.isdigit() else (1, 0, p) for p in pre.split("."))
    return (int(major), int(minor), int(patch), (0, parts))


def update_entry(registry: Path, repository: str, tag: str, packed: dict) -> Path:
    """Create or extend `plugins/<name>.json`; returns the file written."""
    name, version = packed["name"], packed["version"]
    repository = repository.rstrip("/")
    path = registry / "plugins" / f"{name}.json"
    if path.is_file():
        entry = json.loads(path.read_text(encoding="utf-8"))
        if entry["repository"] != repository:
            raise ValueError(f"{name} is registered to {entry['repository']}, not {repository}")
        if any(v["version"] == version for v in entry["versions"]):
            raise ValueError(f"{name} {version} is already in the registry")
    else:
        entry = {"name": name, "repository": repository, "description": None, "versions": []}
    entry["description"] = packed.get("description")
    entry["versions"].append(
        {
            "version": version,
            "url": f"{repository}/releases/download/{tag}/{Path(packed['file']).name}",
            "sha256": packed["sha256"],
            "rpp": packed["rpp"],
        }
    )
    entry["versions"].sort(key=lambda v: semver_key(v["version"]))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(entry, indent=2) + "\n", encoding="utf-8")
    return path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=Path, required=True, help="registry checkout")
    parser.add_argument("--repository", required=True, help="https://github.com/<owner>/<repo>")
    parser.add_argument("--tag", required=True, help="release tag holding the archive")
    parser.add_argument("--packed", type=Path, required=True, help="`rpp plugin pack --json` output")
    args = parser.parse_args()
    packed = json.loads(args.packed.read_text(encoding="utf-8"))
    try:
        path = update_entry(args.registry, args.repository, args.tag, packed)
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
