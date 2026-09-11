"""Bounded subprocess boundary around fatal-capable native window input."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from lifecycle_gate import run
from runtime_evidence import digest, snapshot_executable


def retained_script(name, evidence):
    worker = evidence / name
    identity = evidence / (name + ".sha256")
    if not identity.exists():
        expected = snapshot_executable(Path(__file__).with_name(name), worker)
        identity.write_text(expected)
    elif digest(worker) != identity.read_text():
        raise RuntimeError("retained briefing input executable changed")
    return worker


def run_input(name, arguments, evidence, *, env=None, timeout=10):
    worker = retained_script(name, evidence)
    descriptor, path = tempfile.mkstemp(prefix="briefing-input-" if name == "briefing_x11.py" else "client-input-", suffix=".log", dir=evidence)
    os.close(descriptor)
    log = Path(path)
    try:
        run([sys.executable, worker, *arguments], timeout=timeout, log=log, env=env)
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f"briefing input failed ({error.returncode}); {log}: {log.read_text(errors='replace')}") from error
    return log


def dismiss_briefings(display, evidence, *, timeout=10):
    retained_script("namespace_x11.py", evidence)
    log = run_input("briefing_x11.py", [display], evidence, timeout=timeout)
    # A successful worker reports every matched window, not just the last one.
    windows = json.loads(log.read_text())
    if not isinstance(windows, list) or not windows or any(type(window) is not int or window <= 0 for window in windows):
        raise RuntimeError(f"invalid briefing input result: {log}")
    return windows


def client_key(display, name, evidence):
    # client_x11 imports this confined-X transport, including for keyboard input.
    retained_script("namespace_x11.py", evidence)
    run_input("client_x11.py", ["key", name], evidence,
              env={**os.environ, "DISPLAY": display})
