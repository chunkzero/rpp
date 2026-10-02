#!/usr/bin/env python3
"""Plan immutable releases, verify draft assets, and retain the latest 30 nightlies."""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib

NIGHTLY = re.compile(r"v\d+\.\d+\.\d+-nightly\.\d{8}\.g[0-9a-f]{12}")


def gh(*args):
    return subprocess.check_output(["gh", *args], text=True)


def releases():
    pages = json.loads(
        gh(
            "api",
            "--paginate",
            "--slurp",
            f"repos/{os.environ['GITHUB_REPOSITORY']}/releases",
        )
    )
    return [release for page in pages for release in page]


def published_nightlies(items):
    return sorted(
        (
            item
            for item in items
            if not item["draft"] and NIGHTLY.fullmatch(item["tag_name"])
        ),
        key=lambda item: item["published_at"],
        reverse=True,
    )


def plan(base, sha, event, mode, ref, items, date):
    nightly = event == "schedule" or (
        event == "workflow_dispatch" and mode == "nightly"
    )
    version = f"{base.split('-')[0]}-nightly.{date}.g{sha[:12]}" if nightly else base
    if event == "push" and ref != f"refs/tags/v{base}":
        raise ValueError("release tag must match the project version")
    previous = published_nightlies(items)
    unchanged = (
        event == "schedule"
        and previous
        and f"Source commit: {sha}" in (previous[0]["body"] or "")
    )
    return version, not bool(unchanged)


def publish(directory, version, sha):
    tag = f"v{version}"
    assets = sorted(path for path in directory.iterdir() if path.is_file())
    if not assets:
        raise ValueError("release has no assets")
    for path in assets:
        if path.suffix != ".sha256":
            checksum = path.with_name(path.name + ".sha256")
            if (
                checksum.read_text().split()[0]
                != hashlib.sha256(path.read_bytes()).hexdigest()
            ):
                raise ValueError(f"checksum mismatch: {path.name}")
    existing = next((item for item in releases() if item["tag_name"] == tag), None)
    if existing and (
        f"Source commit: {sha}" not in (existing["body"] or "")
        or existing["target_commitish"] != sha
    ):
        raise ValueError("release version already belongs to another source commit")
    tag_ref = subprocess.run(
        ["git", "ls-remote", "origin", f"refs/tags/{tag}", f"refs/tags/{tag}^{{}}"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    if tag_ref and tag_ref[-1].split()[0] != sha:
        raise ValueError("release tag already belongs to another source commit")
    if existing is None:
        gh(
            "release",
            "create",
            tag,
            "--draft",
            "--target",
            sha,
            "--title",
            tag,
            "--notes",
            f"Source commit: {sha}\n",
        )
    if existing is None or existing["draft"]:
        gh("release", "upload", tag, *(str(path) for path in assets), "--clobber")
    with tempfile.TemporaryDirectory() as temp:
        gh("release", "download", tag, "--dir", temp)
        downloaded = Path(temp)
        if {path.name for path in downloaded.iterdir()} != {
            path.name for path in assets
        }:
            raise ValueError("release asset set differs from the verified package")
        for path in assets:
            if path.read_bytes() != (downloaded / path.name).read_bytes():
                raise ValueError(f"uploaded asset differs: {path.name}")
    if existing is None or existing["draft"]:
        flags = (
            ["--prerelease", "--latest=false"]
            if "-" in version
            else ["--prerelease=false", "--latest"]
        )
        gh("release", "edit", tag, "--draft=false", *flags)
    for item in published_nightlies(releases())[30:]:
        gh("release", "delete", item["tag_name"], "--yes", "--cleanup-tag")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["plan", "publish"])
    parser.add_argument("--directory", type=Path, default=Path("dist"))
    args = parser.parse_args()
    sha = os.environ["GITHUB_SHA"]
    if args.command == "publish":
        publish(args.directory, os.environ["RELEASE_VERSION"], sha)
        return
    with open("Cargo.toml", "rb") as source:
        base = tomllib.load(source)["workspace"]["package"]["version"]
    event = os.environ["GITHUB_EVENT_NAME"]
    version, build = plan(
        base,
        sha,
        event,
        os.environ.get("RELEASE_MODE", "nightly"),
        os.environ["GITHUB_REF"],
        releases() if event == "schedule" else [],
        datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d"),
    )
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"version={version}\nbuild={str(build).lower()}\n")


if __name__ == "__main__":
    main()
