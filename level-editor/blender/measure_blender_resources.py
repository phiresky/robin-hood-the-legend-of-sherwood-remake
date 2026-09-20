"""Sample Linux host pressure and Blender usage without printing command lines."""

import argparse
import json
import os
from pathlib import Path
import time


def snapshot():
    mem = {
        key: int(value.split()[0])
        for key, value in (
            line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines()
        )
    }
    jobs = {}
    for path in Path("/proc").iterdir():
        if not path.name.isdigit():
            continue
        try:
            if path.joinpath("comm").read_text().strip() != "blender":
                continue
            fields = path.joinpath("stat").read_text().rsplit(")", 1)[1].split()
            jobs[path.name] = {
                "cpu_ticks": int(fields[11]) + int(fields[12]),
                "rss_gib": round(int(fields[21]) * os.sysconf("SC_PAGE_SIZE") / 2**30, 3),
                "threads": int(fields[17]),
            }
        except (FileNotFoundError, ProcessLookupError):
            continue
    vm = dict(line.split() for line in Path("/proc/vmstat").read_text().splitlines())
    return {
        "monotonic": time.monotonic(),
        "cpu": list(map(int, Path("/proc/stat").read_text().splitlines()[0].split()[1:9])),
        "memory_gib": {k: round(mem[k] / 2**20, 3) for k in ("MemTotal", "MemAvailable", "SwapTotal", "SwapFree")},
        "swap_pages": {k: int(vm[k]) for k in ("pswpin", "pswpout")},
        "pressure": {k: Path(f"/proc/pressure/{k}").read_text().strip() for k in ("cpu", "memory", "io")},
        "jobs": jobs,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if not 1 <= args.seconds <= 60:
        parser.error("--seconds must be between 1 and 60")
    first = snapshot()
    time.sleep(args.seconds)
    last = snapshot()
    elapsed = last["monotonic"] - first["monotonic"]
    delta = [b - a for a, b in zip(first["cpu"], last["cpu"])]
    for pid, job in last["jobs"].items():
        before = first["jobs"].get(pid)
        job["cpu_cores"] = (
            round((job["cpu_ticks"] - before["cpu_ticks"]) / os.sysconf("SC_CLK_TCK") / elapsed, 2)
            if before else None
        )
    result = {
        "timestamp_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "logical_cpus": os.cpu_count(),
        "sample_seconds": round(elapsed, 2),
        "cpu_busy_percent": round(100 * (sum(delta) - delta[3] - delta[4]) / sum(delta), 1),
        "swap_pages_delta": {k: last["swap_pages"][k] - first["swap_pages"][k] for k in last["swap_pages"]},
        "memory_gib": last["memory_gib"],
        "pressure": last["pressure"],
        "blender_jobs": last["jobs"],
        "process_scope_note": "Run on host; sandbox PID isolation can hide other Blender jobs.",
    }
    output = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(output)
    print(output, end="")


if __name__ == "__main__":
    main()
