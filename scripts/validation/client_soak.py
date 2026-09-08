#!/usr/bin/env python3
"""Run the actual graphical client with bounded duration and isolated writable data.

Usage: client_soak.py BINARY EVIDENCE_DIR DATADIR SECONDS [client arguments...]
DISPLAY must identify an already-running isolated display. Sampling does not
claim GPU memory: /proc RSS is process residency, renderer logs cover its atlas.
Diagnostic-only --replay-export PATH consumes the JSON /get-replay response and
passes its exact compact content through the normal client --replay option.
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

binary, destination, datadir, duration, *arguments = sys.argv[1:]
if "--replay-export" in arguments:
    index = arguments.index("--replay-export")
    exported = json.loads(Path(arguments[index+1]).read_text())
    arguments[index:index+2] = ["--replay", exported["content"]]
destination = Path(destination).resolve()
destination.mkdir(parents=True, exist_ok=True)
environment = dict(os.environ)
for key, directory in {
    "XDG_DATA_HOME": "data", "XDG_CONFIG_HOME": "config",
    "XDG_CACHE_HOME": "cache", "ROBINHOOD_SAVE_DIR": "saves",
}.items():
    path = destination / directory
    path.mkdir(exist_ok=True)
    environment[key] = str(path)
environment.update(ROBINHOOD_DATA_DIR=str(Path(datadir).resolve()),
                   RUST_LOG="debug,wgpu_core=warn,wgpu_hal=warn,naga=warn,reqwest=warn,hyper=warn",
                   RUST_BACKTRACE="1")
command = [str(Path(binary).resolve()), *arguments]
start = time.monotonic()
with (destination / "client.log").open("w") as log, (destination / "samples.jsonl").open("w") as samples:
    process = subprocess.Popen(command, cwd=destination, env=environment,
                               stdout=log, stderr=subprocess.STDOUT)
    (destination / "pid").write_text(str(process.pid))
    print(json.dumps({"pid": process.pid, "command": command, "evidence": str(destination)}), flush=True)
    deadline = start + float(duration)
    bounded_stop = False
    try:
        while process.poll() is None:
            now = time.monotonic()
            if now >= deadline:
                bounded_stop = True
                process.send_signal(signal.SIGTERM)
                break
            try:
                status = Path(f"/proc/{process.pid}/status").read_text()
                fields = dict(line.split(":", 1) for line in status.splitlines() if ":" in line)
                stat = Path(f"/proc/{process.pid}/stat").read_text().split(")", 1)[1].split()
                sample = {"elapsed_s": round(now-start, 3), "rss": fields.get("VmRSS", "").strip(),
                          "hwm": fields.get("VmHWM", "").strip(), "threads": fields.get("Threads", "").strip(),
                          "cpu_ticks": int(stat[11])+int(stat[12]),
                          "fd_count": len(list(Path(f"/proc/{process.pid}/fd").iterdir()))}
                samples.write(json.dumps(sample)+"\n")
                samples.flush()
            except (FileNotFoundError, ProcessLookupError):
                pass
            time.sleep(min(10, max(0.1, deadline-time.monotonic())))
        try:
            code = process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            process.kill()
            code = process.wait()
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=20)
    result = {"command": command, "elapsed_s": round(time.monotonic()-start,3),
              "returncode": code, "bounded_stop": bounded_stop,
              "display": environment.get("DISPLAY"), "datadir": environment["ROBINHOOD_DATA_DIR"],
              "clock_ticks_per_second": os.sysconf("SC_CLK_TCK")}
    (destination / "result.json").write_text(json.dumps(result, indent=2)+"\n")
    print(json.dumps(result), flush=True)
