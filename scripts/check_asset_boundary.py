#!/usr/bin/env python3
"""Assert the resolved pure-content dependency graph, including target edges."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def assert_boundary(output, package, forbidden):
    packages = {line.split()[0] for line in output.splitlines() if line.strip()}
    if package not in packages:
        raise RuntimeError(f"Cargo tree did not include selected package {package}")
    unexpected = packages & forbidden
    if unexpected:
        raise RuntimeError(f"{package} pure codec graph contains {sorted(unexpected)}")


def main():
    for package, forbidden in (
        ("robin_assets", {"robin_engine"}),
        ("robin_content", {"robin_engine", "robin_util", "robin_state_hash_derive", "bitcode"}),
    ):
        command = ["cargo", "tree", "--locked", "-p", package,
                   "--no-default-features", "--edges", "normal,build",
                   "--target", "all", "--prefix", "none", "--format", "{p}"]
        result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, text=True)
        print(result.stdout, end="", flush=True)  # Preserve complete Cargo output.
        result.check_returncode()
        assert_boundary(result.stdout, package, forbidden)


if __name__ == "__main__":
    main()
