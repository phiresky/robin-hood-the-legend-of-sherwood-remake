#!/usr/bin/env python3
"""Bounded, loopback-isolated two-production-process multiplayer diagnostic.

Build separately. Run with `unshare --user --map-root-user --net python3 ...`.
The driver refuses a network namespace with any non-loopback interface.
It never edits game data or reuses a player's save/identity directory.
"""

import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time
import traceback
import urllib.request


from input_worker import dismiss_briefings
from runtime_evidence import DriverInterrupted, failure, finish_children, snapshot_client, stop_process_group, verify_client


ANSI = re.compile(r"\x1b\[[0-9;]*m")



def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--headless", action="store_true")
    parser.add_argument("--mission")
    parser.add_argument("--snapshot", required=True,
                        help="source revision used to build the supplied executable")
    parser.add_argument("--observe-hashes-before-reconnect", action="store_true",
                        help="require a post-bootstrap hash comparison before attempting resynchronization")
    parser.add_argument("--restart-process", action="store_true",
                        help="also diagnose process restart (native transport identity is not durable)")
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    transcript = (evidence / "events.jsonl").open("w", buffering=1)
    children, opened = [], [transcript]
    summary = {"snapshot": args.snapshot, "binary_source": str(args.binary), "data": str(args.data),
               "headless": args.headless, "checks": {}, "completed": False}
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2))
    started = time.monotonic()

    def event(kind, **fields):
        record = {"elapsed_s": round(time.monotonic() - started, 3), "event": kind, **fields}
        transcript.write(json.dumps(record) + "\n")
        print(json.dumps(record), flush=True)

    def wait_for(description, predicate, seconds=60):
        deadline = time.monotonic() + seconds
        latest = None
        while time.monotonic() < deadline:
            try:
                result = predicate()
                if result:
                    event("observed", description=description)
                    return result
            except (OSError, ValueError, TimeoutError) as error:
                latest = str(error)
            time.sleep(0.1)
        raise RuntimeError(f"timed out: {description}; last transient error={latest}")

    def request(port, path, body=None, timeout=5):
        payload = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=payload,
                                     headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return json.load(response)

    def seats(dump):
        if isinstance(dump, dict):
            candidate = dump.get("seats")
            if isinstance(candidate, list) and candidate and "is_lock_alt" in candidate[0]:
                return candidate
            for value in dump.values():
                found = seats(value)
                if found is not None:
                    return found
        elif isinstance(dump, list):
            for value in dump:
                found = seats(value)
                if found is not None:
                    return found
        return None

    def dump_state(label, port):
        result = request(port, "/engine-dump", timeout=15)
        (evidence / f"{label}.engine.json").write_text(json.dumps(result))
        view = seats(result)
        if view is None:
            raise RuntimeError("engine dump omitted deterministic seats")
        event("engine_snapshot", label=label, state=request(port, "/state"), seats=view)
        return view

    def command(port, value):
        result = request(port, "/command", value)
        event("command", port=port, command=value, response=result)
        if "error" in result:
            raise RuntimeError(f"command rejected: {result}")
        return result

    display = None
    def interrupted(signum, _frame):
        raise DriverInterrupted(f"bounded driver interrupted by signal {signum}")
    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGALRM, interrupted)
    signal.alarm(540)
    try:
        binary = snapshot_client(args.binary.resolve(strict=True), evidence, summary)
        data = args.data.resolve(strict=True)
        (evidence / "driver.py").write_bytes(Path(__file__).read_bytes())
        interfaces = json.loads(subprocess.check_output(["ip", "-j", "link", "show"], timeout=10))
        if sorted(interface["ifname"] for interface in interfaces) != ["lo"]:
            raise RuntimeError("refusing non-isolated network namespace")
        subprocess.run(["ip", "link", "set", "lo", "up"], check=True, timeout=10)
        summary["network_interfaces"] = ["lo"]
        if not args.headless:
            read_fd, write_fd = os.pipe()
            logfile = (evidence / "xvfb.log").open("w")
            opened.append(logfile)
            xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0",
                                     "1280x1024x24", "-nolisten", "tcp", "-ac"],
                                    start_new_session=True, pass_fds=(write_fd,), stdout=logfile, stderr=subprocess.STDOUT)
            children.append(xvfb)
            os.close(write_fd)
            with os.fdopen(read_fd) as handle:
                display = ":" + handle.readline().strip()
            event("display", display=display)

        def launch(name, port, connect=None, restart=False):
            peer_root = evidence / ("host" if connect is None else "peer")
            peer_root.mkdir(exist_ok=True)
            for directory in ("save", "config", "cache", "data", "runtime", "cwd"):
                (peer_root / directory).mkdir(exist_ok=True, mode=0o700)
            shutil.copytree(Path(__file__).resolve().parents[2] / "assets/core-datadir",
                            peer_root / "cwd/assets/core-datadir", dirs_exist_ok=True)
            env = os.environ.copy()
            for key in ("ROBINHOOD_OVERLAY_DATA_DIRS", "ROBIN_WAIT_FOR_COMMAND", "WAYLAND_DISPLAY"):
                env.pop(key, None)
            env.update(ROBINHOOD_DATA_DIR=str(data), ROBINHOOD_SAVE_DIR=str(peer_root / "save"),
                       XDG_CONFIG_HOME=str(peer_root / "config"), XDG_CACHE_HOME=str(peer_root / "cache"),
                       XDG_DATA_HOME=str(peer_root / "data"), XDG_RUNTIME_DIR=str(peer_root / "runtime"),
                       RUST_LOG="info,robin_rs::game_session=debug,robin_rs::multiplayer=debug",
                       WGPU_BACKEND="vulkan")
            if display:
                env["DISPLAY"] = display
            argv = [str(binary), "--no-sound", "--mp-nickname", name,
                    "--http-server", str(port), "--mp-browser-join-links", "false"]
            if args.headless:
                argv.append("--headless")
            if args.mission:
                argv += ["--mission", args.mission]
            if connect is None:
                argv += ["--server", "--mp-expected-players", "2", "--record",
                         str(evidence / "host.rhrec.jsonl")]
            else:
                argv += ["--connect", json.dumps(connect)]
            logpath = evidence / ("peer-restarted.log" if restart else f"{name}.log")
            logfile = logpath.open("w")
            opened.append(logfile)
            verify_client(binary, summary)
            process = subprocess.Popen(argv, start_new_session=True, cwd=peer_root / "cwd", env=env,
                                       stdout=logfile, stderr=subprocess.STDOUT)
            children.append(process)
            event("launch", peer=name, pid=process.pid, argv=argv, log=str(logpath))
            return process, logpath

        def log(path):
            return ANSI.sub("", path.read_text(errors="replace"))

        host, host_log = launch("host", 7780)

        def endpoint_id():
            match = re.search(r"hosting on iroh endpoint ([0-9a-f]{64})", log(host_log))
            if host.poll() is not None:
                raise RuntimeError(f"host exited {host.returncode}; inspect {host_log}")
            return match.group(1) if match else None

        host_id = wait_for("host endpoint bound", endpoint_id, 120)
        inodes = set()
        for descriptor in Path(f"/proc/{host.pid}/fd").iterdir():
            try:
                match = re.fullmatch(r"socket:\[(\d+)\]", os.readlink(descriptor))
                if match:
                    inodes.add(match.group(1))
            except FileNotFoundError:
                pass
        ports = set()
        for line in Path(f"/proc/{host.pid}/net/udp").read_text().splitlines()[1:]:
            fields = line.split()
            if fields[9] in inodes:
                ports.add(int(fields[1].split(":")[1], 16))
        if not ports:
            raise RuntimeError("host has no owned IPv4 UDP socket")
        connect = {"id": host_id, "addrs": [{"Ip": f"127.0.0.1:{port}"} for port in sorted(ports)]}
        (evidence / "connect.json").write_text(json.dumps(connect, indent=2))
        peer, peer_log = launch("peer", 7781, connect)
        for name, process, path in (("host", host, host_log), ("peer", peer, peer_log)):
            wait_for(f"{name} BeginSim", lambda path=path: "begin-sim barrier released" in log(path), 120)
            if process.poll() is not None:
                raise RuntimeError(f"{name} exited during admission")
        summary["checks"]["two_production_processes_admitted"] = True
        if display:
            event("synthetic_briefing_return", windows=dismiss_briefings(display, evidence))
        # BeginSim advertises a future wall-clock release; ConnectSeat is a
        # scheduled authoritative input, not guaranteed installed at receipt.
        for port in (7780, 7781):
            wait_for(f"peer {port} advances beyond bootstrap", lambda port=port: request(port, "/state")["frame"] >= 5, 90)
        initial = dump_state("initial-host", 7780)
        dump_state("initial-peer", 7781)
        if len(initial) != 2:
            raise RuntimeError(f"expected exactly two deterministic seats, got {len(initial)}")

        command(7780, {"SetLockAlt": True})
        command(7781, {"SetLockAlt": True})
        command(7781, "SelectAllPcs")
        command(7781, {"AssignQuickGroup": {"index": 0}})
        command(7781, "CrouchDown")
        wait_for("both seat commands committed on host", lambda: [seat["is_lock_alt"] for seat in seats(request(7780, "/engine-dump"))] == [True, True])
        wait_for("peer quick group committed on host", lambda: bool(seats(request(7780, "/engine-dump"))[1]["quick_select_groups"][0]))
        before = dump_state("before-reconnect-host", 7780)
        if not before[1]["selection"] or not before[1]["quick_select_groups"][0]:
            raise RuntimeError("peer selection/quick-group command did not select real PCs")
        summary["checks"]["commands_from_both_seats"] = True

        for attempt in range(4):
            os.kill(host.pid, signal.SIGSTOP)
            try:
                with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
                    pending = executor.submit(command, 7780, {"SetLockAlt": attempt % 2 == 0})
                    time.sleep(0.25 + attempt * 0.1)
                    os.kill(host.pid, signal.SIGCONT)
                    pending.result(timeout=10)
            finally:
                os.kill(host.pid, signal.SIGCONT)
            time.sleep(0.4)
            if "multiplayer rollback timing" in log(peer_log):
                break
        summary["checks"]["late_input_rollback_observed"] = "multiplayer rollback timing" in log(peer_log)
        event("rollback_observation", observed=summary["checks"]["late_input_rollback_observed"])

        if args.observe_hashes_before_reconnect:
            wait_for("pre-reconnect periodic hash agreement", lambda: any(int(frame) > 0 for frame in re.findall(r"multiplayer hash OK frame=(\d+)", log(peer_log))), 90)
            summary["checks"]["pre_reconnect_periodic_hash_agreement"] = True
            dump_state("pre-periodic-host", 7780)
            dump_state("pre-periodic-peer", 7781)

        restarted_log = peer_log
        if args.restart_process:
            stop_process_group(peer)
            children.remove(peer)
            event("peer_process_stopped", returncode=peer.returncode)
            time.sleep(0.5)
            peer, restarted_log = launch("renamed-peer", 7781, connect, restart=True)
            wait_for("replacement process BeginSim", lambda: "begin-sim barrier released" in log(restarted_log), 60)
            if display:
                event("synthetic_replacement_briefing_return", windows=dismiss_briefings(display, evidence))

        start_offset = len(log(restarted_log))
        result = request(7780, "/step-forward", {"n": 4, "synchronized_multiplayer": True}, timeout=20)
        event("synchronized_host_step", response=result)
        if "error" in result:
            raise RuntimeError(f"host synchronized step rejected: {result}")
        wait_for("automatic transport reconnect", lambda: "client reconnected" in log(restarted_log)[start_offset:], 90)
        wait_for("replacement snapshot admission", lambda: "begin-sim barrier released" in log(restarted_log)[start_offset:], 90)
        summary["checks"]["in_process_snapshot_reconnect"] = True
        after = dump_state("post-reconnect-host", 7780)
        dump_state("post-reconnect-peer", 7781)
        if len(after) != 2 or after[1]["selection"] != before[1]["selection"] or after[1]["quick_select_groups"] != before[1]["quick_select_groups"]:
            raise RuntimeError("snapshot reconnect did not retain seat selection/groups")
        summary["checks"]["same_process_seat_preserved"] = True
        command(7781, {"SetLockAlt": False})
        command(7781, "StandUp")
        wait_for("post-reconnect input committed", lambda: seats(request(7780, "/engine-dump"))[1]["is_lock_alt"] is False)
        summary["checks"]["post_reconnect_input"] = True
        dump_state("final-host", 7780)
        dump_state("final-peer", 7781)
        post_input_frame = request(7780, "/state")["frame"]
        event("post_input_hash_boundary", frame=post_input_frame)
        wait_for("post-input periodic hash agreement", lambda: any(int(frame) > post_input_frame for frame in re.findall(r"multiplayer hash OK frame=(\d+)", log(restarted_log)[start_offset:])), 90)
        summary["checks"]["post_input_hash_agreement"] = True
        all_logs = "".join(log(path) for path in dict.fromkeys((host_log, peer_log, restarted_log)))
        summary["hash_ok_count"] = all_logs.count("multiplayer hash OK")
        summary["hash_ok_frames"] = sorted(set(map(int, re.findall(r"multiplayer hash OK frame=(\d+)", all_logs))))
        summary["desync_count"] = all_logs.count("multiplayer DESYNC")
        summary["rollback_count"] = all_logs.count("multiplayer rollback timing")
        summary["missed_hash_comparisons"] = all_logs.count("multiplayer hash comparison missed")
        summary["checks"]["periodic_hash_agreement_observed"] = summary["hash_ok_count"] > 0
        summary["checks"]["no_desync_reported"] = summary["desync_count"] == 0
        summary["completed"] = True
        if not all(summary["checks"].values()):
            raise RuntimeError("one or more live coverage checks were not established")
    except Exception as error:
        failure(summary, error)
        summary["traceback"] = traceback.format_exc()
        event("failure", error=str(error))
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        finish_children(children, summary)
        summary["elapsed_s"] = round(time.monotonic() - started, 3)
        (evidence / "summary.json").write_text(json.dumps(summary, indent=2))
        event("summary", summary=summary)
        for handle in opened:
            handle.close()
    return 0 if summary.get("completed") else 1


if __name__ == "__main__":
    raise SystemExit(main())
