#!/usr/bin/env python3
"""Actual graphical manual stepping/export followed by native replay.

Build separately; run inside a fresh loopback-only user/network namespace.
Every process gets an unused private runtime root. No player saves are touched.
"""

import argparse
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import time
import traceback
import urllib.request

from input_worker import dismiss_briefings
from lifecycle_gate import require_python_xlib
from runtime_evidence import DriverInterrupted, failure, finish_children, snapshot_client, stop_process_group, verify_client


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--snapshot", required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--replay-file", type=Path, help="replay an existing immutable export without repeating live input")
    parser.add_argument("--graphical-replay", action="store_true")
    parser.add_argument("--bootstrap-probe", action="store_true", help="pause playback before admission and retain actual engine state")
    parser.add_argument("--save-load", action="store_true", help="exercise native quicksave/load-back before exporting")
    parser.add_argument("--capture-isolation", action="store_true", help="verify repeated full-map captures preserve paused live rendering and state")
    args = parser.parse_args()
    if args.save_load and args.replay_file:
        parser.error("--save-load requires a fresh live recording")
    if args.save_load and args.bootstrap_probe:
        parser.error("--save-load requires complete playback, not a bootstrap probe")
    if args.capture_isolation and (args.replay_file or args.bootstrap_probe):
        parser.error("--capture-isolation requires a fresh live session and complete playback")
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    summary = {"snapshot": args.snapshot, "binary_source": str(args.binary), "checks": {}, "completed": False}
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2))
    children, streams = [], []

    def interrupted(signum, _frame):
        raise DriverInterrupted(f"bounded driver interrupted by signal {signum}")

    def wait(description, predicate, seconds=90):
        deadline = time.monotonic() + seconds
        last_error = None
        while time.monotonic() < deadline:
            try:
                result = predicate()
                if result:
                    return result
            except (OSError, ValueError, TimeoutError) as error:
                last_error = str(error)
            time.sleep(0.1)
        raise RuntimeError(f"timed out: {description}; {last_error}")

    def request(path, body=None, timeout=30):
        req = urllib.request.Request("http://127.0.0.1:7782" + path,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=timeout) as response:
            result = json.load(response)
        if isinstance(result, dict) and "error" in result:
            raise RuntimeError(f"{path}: {result}")
        return result

    stop = stop_process_group

    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGALRM, interrupted)
    signal.alarm(300)
    try:
        binary = snapshot_client(args.binary.resolve(strict=True), evidence, summary)
        data = args.data.resolve(strict=True)
        require_python_xlib()
        (evidence / "driver.py").write_bytes(Path(__file__).read_bytes())
        interfaces = json.loads(subprocess.check_output(["ip", "-j", "link", "show"], timeout=10))
        if sorted(interface["ifname"] for interface in interfaces) != ["lo"]:
            raise RuntimeError("refusing non-isolated network namespace")
        subprocess.run(["ip", "link", "set", "lo", "up"], check=True, timeout=10)
        read_fd, write_fd = os.pipe()
        xlog = (evidence / "xvfb.log").open("w")
        streams.append(xlog)
        xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0",
            "1280x1024x24", "-nolisten", "tcp", "-ac"], start_new_session=True, pass_fds=(write_fd,),
            stdout=xlog, stderr=subprocess.STDOUT)
        children.append(xvfb)
        os.close(write_fd)
        with os.fdopen(read_fd) as stream:
            display = ":" + stream.readline().strip()

        def launch(name, extra):
            root = evidence / name
            for subdir in ("save", "config", "cache", "data", "runtime", "cwd"):
                (root / subdir).mkdir(parents=True, mode=0o700)
            # A retained executable outside target/ has no installation next
            # to it. Stage the repository's required built-in overlay without
            # making the real install or player runtime writable.
            shutil.copytree(Path(__file__).resolve().parents[2] / "assets/core-datadir",
                root / "cwd/assets/core-datadir")
            env = os.environ.copy()
            for key in ("ROBINHOOD_OVERLAY_DATA_DIRS", "ROBIN_WAIT_FOR_COMMAND", "WAYLAND_DISPLAY"):
                env.pop(key, None)
            env.update(ROBINHOOD_DATA_DIR=str(data), ROBINHOOD_SAVE_DIR=str(root / "save"),
                XDG_CONFIG_HOME=str(root / "config"), XDG_CACHE_HOME=str(root / "cache"),
                XDG_DATA_HOME=str(root / "data"), XDG_RUNTIME_DIR=str(root / "runtime"),
                DISPLAY=display, WGPU_BACKEND="vulkan", RUST_LOG="info,robin_rs::game_session=debug")
            logfile = (evidence / f"{name}.log").open("w")
            streams.append(logfile)
            argv = [str(binary), "--no-sound", "--http-server", "7782", *extra]
            verify_client(binary, summary)
            child = subprocess.Popen(argv, start_new_session=True, cwd=root / "cwd", env=env,
                stdout=logfile, stderr=subprocess.STDOUT)
            children.append(child)
            summary[name + "_argv"] = argv
            return child

        if args.replay_file:
            replay = args.replay_file.resolve(strict=True)
            summary["replay_input"] = str(replay)
        else:
            live = launch("live", ["--record", str(evidence / "live.rhrec.jsonl")])
            def live_state():
                if live.poll() is not None:
                    raise RuntimeError(f"live game exited {live.returncode}")
                # The initial briefing precedes mission RPC draining. Dismiss
                # actual setup UI before waiting on its first engine response.
                try:
                    dismiss_briefings(display, evidence)
                except RuntimeError as error:
                    if "no real Robin windows" not in str(error):
                        raise
                return request("/state", timeout=1)
            wait("live engine RPC", live_state)
            summary["windows"] = dismiss_briefings(display, evidence)
            last_dismissal = time.monotonic()
            def ordinary_ticks():
                nonlocal last_dismissal
                if live.poll() is not None:
                    raise RuntimeError(f"live game exited {live.returncode}")
                if request("/state")["frame"] >= 5:
                    return True
                if time.monotonic() - last_dismissal > 1:
                    dismiss_briefings(display, evidence)
                    last_dismissal = time.monotonic()
                return False
            wait("ordinary graphical ticks", ordinary_ticks)
            summary["before_unpaused_step"] = request("/state")
            summary["unpaused_step"] = request("/step-forward", {"n": 4})
            assert summary["unpaused_step"]["advanced"] == 4
            summary["checks"]["normal_frame_and_manual_steps"] = True
            summary["pause"] = request("/set-paused", {"paused": True})
            summary["paused_step"] = request("/step-forward", {"n": 30, "auto_dismiss": True})
            assert summary["paused_step"]["advanced"] == 30
            summary["checks"]["paused_manual_steps"] = True
            if args.save_load:
                from save_load_live import exercise_save_load
                (evidence / "save_load_live.py").write_bytes(Path(__file__).with_name("save_load_live.py").read_bytes())
                exercise_save_load(request, wait, display, evidence, summary)
            summary["live_state"] = request("/state")
            if args.capture_isolation:
                from capture_live import exercise_capture
                (evidence / "capture_live.py").write_bytes(Path(__file__).with_name("capture_live.py").read_bytes())
                def capture(query):
                    with urllib.request.urlopen("http://127.0.0.1:7782/screenshot?" + query, timeout=60) as response:
                        return response.read()
                exercise_capture(request, capture, evidence, summary)
            with urllib.request.urlopen("http://127.0.0.1:7782/screenshot", timeout=30) as response:
                screenshot = response.read()
            assert screenshot.startswith(b"\x89PNG\r\n\x1a\n")
            (evidence / "live.png").write_bytes(screenshot)
            exported = request("/get-replay")
            (evidence / "export.json").write_text(json.dumps(exported))
            content = exported["content"]
            assert content.startswith("rhrec-")
            replay = evidence / "export.rhrec"
            replay.write_text(content)
            summary["checks"]["canonical_compact_export"] = True
            stop(live)
            children.remove(live)
        playback = launch("playback", ["--replay", str(replay),
            "--fast-forward" if args.graphical_replay else "--headless",
            *(["--start-paused"] if args.bootstrap_probe else [])])
        logpath = evidence / "playback.log"
        if args.bootstrap_probe:
            wait("paused bootstrap RPC", lambda: request("/state", timeout=1))
            (evidence / "bootstrap.engine.json").write_text(json.dumps(request("/engine-dump")))
            summary["checks"]["paused_bootstrap_dump"] = True
            summary["completed"] = True
            return 0
        last_dismissal = 0
        def replay_finished():
            nonlocal last_dismissal
            log = logpath.read_text(errors="replace")
            if "headless replay finished" in log or "Replay finished after" in log:
                return True
            if playback.poll() is not None:
                raise RuntimeError(f"replay exited {playback.returncode}; inspect {logpath}")
            if args.graphical_replay and time.monotonic() - last_dismissal > 1:
                try:
                    dismiss_briefings(display, evidence)
                except RuntimeError as error:
                    if "no real Robin windows" not in str(error):
                        raise
                last_dismissal = time.monotonic()
            return False
        wait("complete native replay", replay_finished)
        log = logpath.read_text(errors="replace")
        assert "Replay desync" not in log
        assert "Replay hash OK @ frame 25" in log
        if args.save_load:
            from save_load_live import verify_replay
            verify_replay(log, summary["save_load"]["load_record_frame"], summary["save_load"]["final_record_frame"])
            summary["checks"]["save_load_post_restore_replay_hashes"] = True
        summary["checks"]["native_playback_finished"] = True
        summary["checks"]["post_bootstrap_hash_verified"] = True
        summary["completed"] = True
    except Exception as error:
        failure(summary, error)
        summary["traceback"] = traceback.format_exc()
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        finish_children(children, summary)
        (evidence / "summary.json").write_text(json.dumps(summary, indent=2))
        for stream in streams:
            stream.close()
        print(json.dumps(summary, indent=2), flush=True)
    return 0 if summary.get("completed") else 1


if __name__ == "__main__":
    raise SystemExit(main())
