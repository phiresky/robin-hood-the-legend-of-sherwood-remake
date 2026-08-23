"""Summarize a parity trace corpus: schema, seed, mission and session per trace.

    scripts/summarize_parity_traces.py <corpus-traces-dir> <output.json>

Reads only each trace's header line by default, which is cheap. DEEP=1 also
decompresses every trace in full to report the terminator record, the recorded
frame count, and how many lines carry an audio RNG domain or a refused-action
command -- hundreds of megabytes per trace, so reach for it on a sample rather
than a corpus. A capture that produced a .complete marker already had its
rng_suffix terminator checked at capture time.

JOBS controls parallelism (default 6).

This is a capture-time tool: it reads the JSONL recordings themselves, so run
it before recordings are converted into native .parity.bitcode.zst artifacts
(conversion deletes the JSONL).
"""

import json
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor


def first_line(path):
    # Stop zstd once the header is out instead of decompressing the whole trace.
    process = subprocess.Popen(
        ["zstd", "-dc", "--long=31", path],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    try:
        return process.stdout.readline()
    finally:
        process.stdout.close()
        process.kill()
        process.wait()


def count_matching(path, pattern):
    completed = subprocess.run(
        f"zstd -dc --long=31 {path!r} | grep -c {pattern!r} || true",
        shell=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return int(completed.stdout.strip() or 0)


def last_line(path):
    completed = subprocess.run(
        f"zstd -dc --long=31 {path!r} | tail -n 1",
        shell=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    return completed.stdout


def summarize(root, path):
    try:
        header = json.loads(first_line(path))
    except Exception as error:
        return {"path": os.path.relpath(path, root), "error": f"header:{type(error).__name__}"}

    record = {
        "path": os.path.relpath(path, root),
        "schema": header.get("schema"),
        "random_input_seed": header.get("random_input_seed"),
        "rng_seed": header.get("rng_seed"),
        "mission": header.get("mission"),
        "session_index": header.get("session_index"),
        "initial_frame": header.get("initial_frame"),
    }
    if os.environ.get("DEEP") == "1":
        try:
            terminator = json.loads(last_line(path))
            record["terminator"] = terminator.get("type")
            record["frame_count"] = terminator.get("frame_count")
            record["final_frame"] = terminator.get("final_frame")
        except Exception as error:
            record["terminator"] = f"error:{type(error).__name__}"
        record["audio_domain_lines"] = count_matching(path, '"audio"')
        record["refused_action_lines"] = count_matching(path, "hero_refused_action")
    return record


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: summarize_parity_traces.py <corpus-traces-dir> <output.json>")
    root, output_path = sys.argv[1], sys.argv[2]

    traces = sorted(
        os.path.join(directory, name)
        for directory, _, names in os.walk(root)
        for name in names
        if "-session-" in name and name.endswith(".jsonl.zst")
    )
    jobs = int(os.environ.get("JOBS", "6"))
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        records = list(pool.map(lambda path: summarize(root, path), traces))
    with open(output_path, "w") as handle:
        json.dump(records, handle, indent=1)
    print(f"{len(records)} traces -> {output_path}")


if __name__ == "__main__":
    main()
