#!/usr/bin/env python3
"""Assert content and offline-tool dependency boundaries, including target edges."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def assert_boundary(output, package, forbidden):
    packages = {line.split()[0] for line in output.splitlines() if line.strip()}
    if package not in packages:
        raise RuntimeError(f"Cargo tree did not include selected package {package}")
    unexpected = packages & forbidden
    if unexpected:
        raise RuntimeError(f"{package} dependency graph contains {sorted(unexpected)}")


def main():
    for package, forbidden, features in (
        ("robin_level_data", {"robin_engine", "robin_rs", "robin_assets", "robin_spellforge"}, []),
        ("robin_legacy_save", {"robin_engine", "robin_rs", "robin_assets", "robin_spellforge"}, []),
        ("robin_engine_types", {"robin_engine", "robin_rs", "robin_assets", "robin_spellforge"}, []),
        # The simulation layer uses plain run-identity types, never the signed
        # leaderboard protocol or its Ed25519 implementation.
        ("robin_engine", {"robin_run_protocol", "ed25519-dalek", "robin_rs", "robin_assets"}, []),
        ("robin_run_types", {"robin_run_protocol", "robin_engine", "robin_util", "ed25519-dalek"}, []),
        ("robin_assets", {"robin_engine"}, []),
        ("robin_asset_codecs", {"robin_engine", "robin_assets"}, []),
        ("robin_script_types", {"robin_engine", "robin_spellforge"}, []),
        ("robin_spellforge", {"robin_engine"}, []),
        ("robin_modding_tools", {"robin_rs"}, []),
        ("robin_replay_format", {"robin_rs", "robin_assets", "wgpu", "winit", "kira", "cpal", "ffmpeg-next"}, []),
    ):
        command = ["cargo", "tree", "--locked", "-p", package,
                   "--no-default-features", "--edges", "normal,build",
                   "--target", "all", "--prefix", "none", "--format", "{p}", *features]
        result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, text=True)
        print(result.stdout, end="", flush=True)  # Preserve complete Cargo output.
        result.check_returncode()
        assert_boundary(result.stdout, package, forbidden)


if __name__ == "__main__":
    main()
