#!/usr/bin/env python3
"""Bounded read-only HTTP frame sampling; errors/stalls remain visible.

Usage: client_frames.py URL EVIDENCE_JSONL SECONDS
Run alongside client_soak.py; /host-debug is a non-mutating snapshot.
"""
import json
from pathlib import Path
import sys
import time
import urllib.request

url, output, duration = sys.argv[1:]
start = time.monotonic()
deadline = start + float(duration)
with Path(output).open("w") as destination:
    while time.monotonic() < deadline:
        before = time.monotonic()
        sample = {"elapsed_s": round(before-start, 3)}
        try:
            with urllib.request.urlopen(url, timeout=4) as response:
                state = json.load(response)
            sample.update(frame=state.get("frame"), selection=state.get("selection"),
                          selected_pc=state.get("selected_pc"), error=state.get("error"))
        except Exception as error:
            sample["error"] = str(error)
        sample["request_s"] = round(time.monotonic()-before, 3)
        destination.write(json.dumps(sample)+"\n")
        destination.flush()
        time.sleep(min(10, max(0.1, deadline-time.monotonic())))
