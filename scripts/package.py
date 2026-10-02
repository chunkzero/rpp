#!/usr/bin/env python3
"""Package the native RPP binary with its checksum-pinned TypeScript compiler."""

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
from urllib.request import urlopen

TYPESCRIPT_VERSION = "7.0.2"
TYPESCRIPT_SHA256 = "da2513f4b95176d6dde8b51aab7afe8a927656c9d277369793f77f7e59371c08"
NATIVE_SHA256 = {
    "linux-x64": "7ecad6f67377e831856367ab062ef394f21506a611405bf8ac0ff039348637d3",
    "linux-arm64": "c83d931ac9dd7549cde6e71246aa9d6a9812843023df3e277fe3b5dcf41dd0ea",
    "darwin-arm64": "902e2fe1cf0799198ef902c6b8c310a450fef629a6baba41d45641ef75c04ebd",
    "darwin-x64": "eba158cb54050f723d5ff781438f33de5640054440bb4f2bd170cfe9bc2eb551",
    "win32-x64": "61fc4e141d2bc687db580e71bbfa63b9c209f0310645d82ca1b457eb3a24fd19",
}


def host_platform():
    system = {"Linux": "linux", "Darwin": "darwin", "Windows": "win32"}[platform.system()]
    arch = {"x86_64": "x64", "AMD64": "x64", "arm64": "arm64", "aarch64": "arm64"}[platform.machine()]
    return f"{system}-{arch}"


def fetch(package, checksum, destination):
    name = package.rsplit("/", 1)[-1]
    url = f"https://registry.npmjs.org/{package}/-/{name}-{TYPESCRIPT_VERSION}.tgz"
    with urlopen(url, timeout=60) as response:
        content = response.read()
    if hashlib.sha256(content).hexdigest() != checksum:
        raise ValueError(f"checksum mismatch: {package}")
    with tarfile.open(fileobj=io.BytesIO(content), mode="r:gz") as archive:
        archive.extractall(destination, filter="data")
    return destination / "package"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("target/dist"))
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[1]
    os.chdir(repository)
    target = host_platform()
    if target not in NATIVE_SHA256:
        raise ValueError(f"unsupported release platform: {target}")
    with open("Cargo.toml", "rb") as source:
        version = os.environ.get("RPP_RELEASE_VERSION") or tomllib.load(source)["workspace"]["package"]["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?", version):
        raise ValueError(f"invalid release version: {version}")
    suffix = ".exe" if target.startswith("win32-") else ""
    executable = repository / f"target/release/rpp{suffix}"
    actual = subprocess.check_output([executable, "--version"], text=True).strip()
    if actual != f"rpp {version}":
        raise ValueError(f"expected rpp {version}, got {actual}")
    name = f"rpp-{version}-{target.replace('win32-', 'windows-')}"
    args.output.mkdir(parents=True, exist_ok=True)
    output = args.output.resolve()
    archive = output / f"{name}.tar.gz"
    checksum = archive.with_name(archive.name + ".sha256")
    if archive.exists() or checksum.exists():
        raise ValueError(f"refusing to replace {archive}")
    with tempfile.TemporaryDirectory() as temporary:
        work = Path(temporary)
        typescript = fetch("typescript", TYPESCRIPT_SHA256, work / "typescript")
        native = fetch(f"@typescript/typescript-{target}", NATIVE_SHA256[target], work / "native")
        root = work / name
        compiler = root / "toolchain/typescript" / TYPESCRIPT_VERSION
        shutil.copytree(native / "lib", compiler)
        shutil.copy2(executable, root / f"rpp{suffix}")
        for filename in ("LICENSE-MIT", "LICENSE-APACHE"):
            shutil.copy2(repository / filename, root / filename)
        for filename in ("LICENSE", "NOTICE.txt"):
            shutil.copy2(typescript / filename, compiler / filename)
        sha = os.environ.get("GITHUB_SHA") or subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
        (root / "release.json").write_text(json.dumps({"version": version, "source": sha}) + "\n")
        # Fixed metadata keeps the archive independent of runner paths and timestamps.
        with archive.open("xb") as file, gzip.GzipFile(filename="", mode="wb", fileobj=file, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as tar:
                for path in [root, *sorted(root.rglob("*"))]:
                    entry = tar.gettarinfo(str(path), arcname=str(path.relative_to(work)))
                    entry.uid = entry.gid = entry.mtime = 0
                    entry.uname = entry.gname = ""
                    entry.pax_headers = {}
                    entry.mode = 0o755 if path.is_dir() or path.name in (f"rpp{suffix}", f"tsc{suffix}") else 0o644
                    if path.is_file():
                        with path.open("rb") as content:
                            tar.addfile(entry, content)
                    else:
                        tar.addfile(entry)
    checksum.write_text(f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
    print(archive)


if __name__ == "__main__":
    main()
