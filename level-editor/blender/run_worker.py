"""Run a refinement job in its own Blender file and output directory.

Example (from the repository root):
  blender --background --factory-startup --python-exit-code 1 \
    --python level-editor/blender/run_worker.py -- \
    --source level-editor/work/derby-refinement/derby-refinement.blend \
    --job path/to/refinement_job.py --output-dir path/to/new/worker-directory

The job receives ``worker_output_dir`` and ``worker_source_blend`` globals and
may set a JSON-serializable ``result``. It must edit the loaded copy only. This
entrypoint saves worker.blend after success; nothing is merged into the source.
Use --threads to limit each worker's rendering load when running many workers.
"""

import argparse
import contextlib
import hashlib
import json
from pathlib import Path
import runpy
import shutil
import sys
import traceback

import bpy


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--job", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--threads", type=int, default=2)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
    source = args.source.resolve(strict=True)
    job = args.job.resolve(strict=True)
    output = args.output_dir.resolve()
    if source.suffix != ".blend" or job.suffix != ".py":
        parser.error("--source must be a .blend file and --job a Python script")
    if args.threads < 1:
        parser.error("--threads must be positive")
    # An exclusive directory prevents concurrent workers from mixing artifacts.
    output.mkdir(parents=True, exist_ok=False)
    working = output / "worker.blend"
    source_hash = digest(source)
    shutil.copy2(source, working)
    report = {
        "source": str(source), "source_sha256": source_hash,
        "job": str(job), "job_sha256": digest(job),
        "output_blend": str(working), "blender": bpy.app.version_string,
        "status": "running",
    }
    try:
        with (output / "job.log").open("x") as log:
            with contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
                bpy.ops.wm.open_mainfile(filepath=str(working), load_ui=False)
                for scene in bpy.data.scenes:
                    scene.render.threads_mode = "FIXED"
                    scene.render.threads = args.threads
                # Resolve images against the source before saving the relocated copy.
                for image in bpy.data.images:
                    if image.filepath and not image.packed_file:
                        image.filepath = bpy.path.abspath(
                            image.filepath, start=str(source.parent))
                namespace = runpy.run_path(str(job), init_globals={
                    "worker_output_dir": str(output),
                    "worker_source_blend": str(source),
                }, run_name="__main__")
                report["result"] = namespace.get("result")
                # Fail before saving if the job returned unsupported report data.
                json.dumps(report)
                bpy.ops.wm.save_as_mainfile(filepath=str(working))
                report["status"] = "complete"
    except BaseException:
        report["status"] = "failed"
        report["error"] = traceback.format_exc()
        raise
    finally:
        report["source_unchanged"] = digest(source) == source_hash
        (output / "worker.json").write_text(json.dumps(report, indent=2) + "\n")
    if not report["source_unchanged"]:
        raise RuntimeError("Source blend changed during this worker; check concurrent writes")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
