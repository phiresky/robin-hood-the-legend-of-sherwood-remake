#!/usr/bin/env python3
"""Publish only a complete, hash-verified draft; existing releases are immutable.

This command mutates GitHub only when explicitly executed by the release job.
Interrupted drafts can be resumed with the same tag and identical local bytes.
No delete, asset replacement, or force-tag operation is performed.
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess


def inventory(root: Path) -> dict[str, tuple[Path, str]]:
    assets = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"release input contains a symlink: {path}")
        if not path.is_file():
            continue
        if path.name in assets:
            raise ValueError(f"duplicate release asset name: {path.name}")
        if path.stat().st_size == 0:
            raise ValueError(f"empty release asset: {path}")
        with path.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        assets[path.name] = (path, digest)
    required = {"robin-windows-x86_64.zip", "robin-linux-x86_64.tar.gz"}
    if not required.issubset(assets):
        raise ValueError(f"missing platform artifacts: {sorted(required - assets.keys())}")
    # Both platform update indexes must name a complete set of packages.
    for runtime in ("win", "linux"):
        index_name = f"releases.{runtime}.json"
        if index_name not in assets:
            raise ValueError(f"missing Velopack index: {index_name}")
        index = json.loads(assets[index_name][0].read_text())
        entries = index.get("Assets")
        if not isinstance(entries, list) or not entries:
            raise ValueError(f"empty/invalid Velopack index: {index_name}")
        for entry in entries:
            name = entry.get("FileName")
            if not isinstance(name, str) or name not in assets:
                raise ValueError(f"{index_name} references missing asset {name!r}")
            path, digest = assets[name]
            if entry.get("Size") != path.stat().st_size or str(entry.get("SHA256", "")).lower() != digest:
                raise ValueError(f"{index_name} size/hash differs for {name}")
    return assets


def gh(*args: str, binary: bool = False):
    result = subprocess.run(["gh", *args], check=True, stdout=subprocess.PIPE)
    return result.stdout if binary else result.stdout.decode()


def release_tag(event: str, ref: str, run_id: str, date: str) -> tuple[str, bool]:
    if event in ("schedule", "workflow_dispatch"):
        if not run_id.isdecimal():
            raise ValueError("GITHUB_RUN_ID must be numeric")
        return f"nightly-{date}-{run_id}", True
    if not ref.startswith("v"):
        raise ValueError("stable releases require a version tag")
    return ref, False


def run_date(repo: str, run_id: str) -> str:
    # A rerun tomorrow must resume today's draft, not choose a new tag.
    run = json.loads(gh("api", f"repos/{repo}/actions/runs/{run_id}"))
    return datetime.datetime.fromisoformat(run["created_at"]).date().isoformat()


def find_release(repo: str, tag: str):
    # Authentication/network failures must propagate, never mean 'not found'.
    pages = json.loads(gh("api", "--paginate", "--slurp", f"repos/{repo}/releases?per_page=100"))
    return next((release for page in pages for release in page if release["tag_name"] == tag), None)


def remote_asset_sha256(repo: str, asset_id: int) -> str:
    command = ["gh", "api", "-H", "Accept: application/octet-stream",
               f"repos/{repo}/releases/assets/{asset_id}"]
    with subprocess.Popen(command, stdout=subprocess.PIPE) as process:
        assert process.stdout is not None
        digest = hashlib.file_digest(process.stdout, "sha256").hexdigest()
        if process.wait() != 0:
            raise subprocess.CalledProcessError(process.returncode, command)
    return digest


def verify_remote(repo: str, release: dict, assets: dict, *, allow_missing: bool):
    remote = {asset["name"]: asset for asset in release["assets"]}
    extras = remote.keys() - assets.keys()
    if extras:
        raise ValueError(f"release contains unexpected assets: {sorted(extras)}")
    missing = assets.keys() - remote.keys()
    if missing and not allow_missing:
        raise ValueError(f"release is missing assets: {sorted(missing)}")
    for name, asset in remote.items():
        # Download and hash bytes rather than trusting a mutable release label
        # or assuming GitHub's optional digest metadata is available.
        if remote_asset_sha256(repo, asset['id']) != assets[name][1]:
            raise ValueError(f"immutable release asset differs: {name}; use a new candidate tag")
    return sorted(missing)


def publish(root: Path, repo: str, tag: str, commit: str, prerelease: bool):
    assets = inventory(root)
    release = find_release(repo, tag)
    if release is None:
        gh("release", "create", tag, "--repo", repo, "--draft", "--target", commit,
           "--title", tag, "--notes", f"Build of {commit}. Assets verified before publication.",
           *(["--prerelease"] if prerelease else ["--verify-tag"]))
        release = find_release(repo, tag)
        if release is None:
            raise RuntimeError("created draft is not visible; retry without deleting the candidate")
    if release["target_commitish"] != commit:
        raise ValueError("release target differs from the requested commit")
    missing = verify_remote(repo, release, assets, allow_missing=release["draft"])
    if not release["draft"]:
        print(f"{tag} is already published with identical assets")
        return
    for name in missing:
        gh("release", "upload", tag, str(assets[name][0]), "--repo", repo)
    release = find_release(repo, tag)
    if release is None:
        raise RuntimeError("candidate disappeared before verification")
    verify_remote(repo, release, assets, allow_missing=False)
    gh("release", "edit", tag, "--repo", repo, "--draft=false")
    print(f"published verified candidate {tag}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("assets", type=Path)
    args = parser.parse_args()
    repo = os.environ["GITHUB_REPOSITORY"]
    event = os.environ["GITHUB_EVENT_NAME"]
    created_date = run_date(repo, os.environ["GITHUB_RUN_ID"]) if event in ("schedule", "workflow_dispatch") else ""
    tag, prerelease = release_tag(os.environ["GITHUB_EVENT_NAME"], os.environ["GITHUB_REF_NAME"],
                                  os.environ["GITHUB_RUN_ID"], created_date)
    publish(args.assets, repo, tag, os.environ["GITHUB_SHA"], prerelease)
