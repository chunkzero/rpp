#!/usr/bin/env python3
"""Verify the packaged executable, bundled compiler, example pack, and WASIp2 consumer."""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

from package import host_platform


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("version")
    args = parser.parse_args()
    archive = args.archive.resolve()
    expected = archive.with_name(archive.name + ".sha256").read_text().split()[0]
    if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
        raise ValueError("package checksum mismatch")
    repository = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        if os.name == "nt":
            with tarfile.open(archive) as package:
                package.extractall(root, filter="data")
            executable = root / archive.name.removesuffix(".tar.gz") / "rpp.exe"
        else:
            prefix = root / "prefix"
            env = dict(os.environ, RPP_INSTALL_DIR=str(prefix))
            subprocess.run(["sh", repository / "scripts/install.sh", args.version, archive], env=env, check=True)
            executable = prefix / "bin/rpp"
            corrupted = root / archive.name
            shutil.copy2(archive, corrupted)
            corrupted.with_name(corrupted.name + ".sha256").write_text(f"{'0' * 64}  {corrupted.name}\n")
            result = subprocess.run(["sh", repository / "scripts/install.sh", args.version, corrupted], env=dict(env, RPP_INSTALL_DIR=str(root / "rejected")))
            if result.returncode == 0 or (root / "rejected/bin/rpp").exists():
                raise ValueError("installer accepted a corrupted archive")
        actual = subprocess.check_output([executable, "--version"], text=True).strip()
        if actual != f"rpp {args.version}":
            raise ValueError(f"unexpected version: {actual}")
        # An empty PATH and no override prove checks use the compiler shipped in the archive.
        env = dict(os.environ, PATH="")
        env.pop("RPP_TSC", None)
        plugin = root / "check-plugin"
        shutil.copytree(repository / "scripts/fixtures/check-plugin", plugin)
        subprocess.run([executable, "-C", plugin, "check"], env=env, check=True)
        component = repository / "examples/plugins/grayscale-wasm/grayscale.wasm"
        if not component.is_file():
            raise ValueError(f"missing {component}; build it with `just example-wasm`")
        examples = root / "examples"
        artifacts = shutil.ignore_patterns(".rpp", "dist", "generated", "target", "plugin.wasm")
        shutil.copytree(repository / "examples", examples, ignore=artifacts)
        grayscale = root / "grayscale"
        shutil.copytree(repository / "scripts/fixtures/grayscale", grayscale)
        for project in (examples / "pack", grayscale):
            for command in ["check", "build"]:
                subprocess.run([executable, "-C", project, command], env=env, check=True)
        if not (examples / "pack/dist/rpp-example-pack.zip").is_file():
            raise ValueError("example pack build did not write its zip")
        # Byte 25 of a PNG is the IHDR color type; 0 is grayscale.
        texture = grayscale / "dist/assets/minecraft/textures/block/stone.png"
        if texture.read_bytes()[25] != 0:
            raise ValueError("grayscale component did not convert the texture")
        print(f"Verified {host_platform()} consumer")


if __name__ == "__main__":
    main()
