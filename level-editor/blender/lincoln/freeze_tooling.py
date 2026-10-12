"""Freeze or verify a content-addressed snapshot of shared refinement helpers."""
import argparse
import hashlib
import json
from pathlib import Path
import sys


EDITOR = Path(__file__).resolve().parents[2]
DEFAULT_ROOT = EDITOR / "work/needs-review/lincoln-refinement/tooling"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def select_tooling(directory=None):
    """Verify all helper bytes, reject previously imported live helpers, then pin imports."""
    if directory is None:
        pointer = json.loads((DEFAULT_ROOT / "current.json").read_text())
        directory = pointer["directory"]
    directory = Path(directory).resolve(strict=True)
    manifest_path = directory / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("version") != 1 or not manifest.get("files"):
        raise ValueError("Invalid tooling snapshot manifest")
    actual = {path.name for path in directory.glob("*.py")}
    if actual != set(manifest["files"]):
        raise ValueError("Tooling snapshot file set changed")
    for name, expected in manifest["files"].items():
        if Path(name).name != name or sha(directory / name) != expected:
            raise ValueError(f"Tooling snapshot changed: {name}")
        imported = sys.modules.get(Path(name).stem)
        if imported is not None and getattr(imported, "__file__", None):
            if Path(imported.__file__).resolve() != directory / name:
                raise RuntimeError(f"Live helper already imported before snapshot selection: {name}")
    sys.path.insert(0, str(directory))
    return {"directory": str(directory), "snapshot_id": directory.name,
            "manifest_sha256": sha(manifest_path), "files": manifest["files"]}


def freeze(source, destination):
    source, destination = Path(source).resolve(strict=True), Path(destination).resolve()
    # Capture once: another session may be editing the source tree concurrently.
    directories = [source]
    shared = EDITOR / 'refinement/blender'
    if source in ((EDITOR / 'blender').resolve(), shared.resolve()):
        directories = [EDITOR / 'blender', shared]
    paths = {path.name: path for directory in directories for path in sorted(directory.glob('*.py'))}
    captured = {name: path.read_bytes() for name, path in paths.items()}
    if not captured:
        raise ValueError("No Python helper files found")
    files = {name: hashlib.sha256(data).hexdigest() for name, data in captured.items()}
    identifier = hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()[:16]
    snapshot = destination / identifier
    if not snapshot.exists():
        snapshot.mkdir(parents=True)
        for name, data in captured.items():
            (snapshot / name).write_bytes(data)
        (snapshot / "manifest.json").write_text(json.dumps({"version": 1, "files": files,
            'sources': {name: str(path.resolve()) for name, path in paths.items()}}, indent=2) + "\n")
    evidence = select_tooling(snapshot)
    pointer = destination / "current.json"
    temporary = destination / ".current.json.tmp"
    temporary.write_text(json.dumps({"directory": str(snapshot), "snapshot_id": identifier}, indent=2) + "\n")
    temporary.replace(pointer)
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=EDITOR / "refinement/blender")
    parser.add_argument("--output", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--verify", type=Path, help="Verify an existing snapshot instead of freezing current helpers")
    args = parser.parse_args()
    print(json.dumps(select_tooling(args.verify) if args.verify else freeze(args.source, args.output), indent=2))


if __name__ == "__main__":
    main()
