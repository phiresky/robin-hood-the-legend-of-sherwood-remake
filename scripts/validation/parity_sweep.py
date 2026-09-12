#!/usr/bin/env python3
"""Isolated, bounded diagnostic corpus sampling; never modifies source campaigns."""
from __future__ import annotations

import argparse
import collections
import concurrent.futures
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import struct
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from parity_result import exact_eof, read_result

SUFFIX = ".parity.bitcode.zst"
FOOTER = struct.Struct("<16sIQQ")
MAGIC = b"RHPRTRACEFOOTER!"


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def extent(path):
    with path.open("rb") as stream:
        stream.seek(-FOOTER.size, 2)
        magic, version, frames, final = FOOTER.unpack(stream.read())
    if magic != MAGIC or version not in (66, 67, 68):
        raise ValueError(f"unsupported native footer: {magic!r}, {version}")
    return dict(native_version=version, frames=frames, final_frame=final)


def identity(relative):
    parts = relative.parts
    campaign = "/".join(parts[:2]) if parts[0] == "60s-random-input" else parts[0]
    if "traces" in parts:
        source = list(parts[parts.index("traces") + 1:])
        if len(source) == 3:
            source[-1] = source[-1].split("-session-")[0]
        else:
            source = source[:-1]
        save = "/".join(source)
    else:
        save = relative.name.rsplit("-session-", 1)[0]
    return campaign, save


def inventory(corpus):
    entries, errors = [], []
    for path in sorted(corpus.rglob("*" + SUFFIX)):
        relative = path.relative_to(corpus)
        try:
            if path.is_symlink():
                raise ValueError("source trace is a symlink")
            campaign, save = identity(relative)
            entries.append(dict(path=str(relative), campaign=campaign, save=save,
                                bytes=path.stat().st_size, **extent(path)))
        except (OSError, ValueError, struct.error) as error:
            errors.append(dict(path=str(relative), error=str(error)))
    groups = collections.defaultdict(list)
    for entry in entries:
        groups[entry["campaign"]].append(entry)
    summary = dict(native_files=len(entries) + len(errors), readable_footers=len(entries),
                   bytes=sum(e["bytes"] for e in entries), errors=errors, campaigns={})
    for campaign, group in sorted(groups.items()):
        summary["campaigns"][campaign] = dict(
            traces=len(group), saves=len({e["save"] for e in group}),
            frames=sum(e["frames"] for e in group),
            min_frames=min(e["frames"] for e in group),
            max_frames=max(e["frames"] for e in group),
            native_versions=dict(collections.Counter(e["native_version"] for e in group)))
    return entries, summary


def choose(entries, count):
    """Breadth-first across campaigns and stable-hash-ordered save identities."""
    groups = collections.defaultdict(list)
    for entry in entries:
        groups[entry["campaign"]].append(entry)
    queues = {}
    for campaign, group in sorted(groups.items()):
        if campaign == "interactive" or len(group) <= 3:
            queues[campaign] = sorted(group, key=lambda e: (-e["frames"], e["path"]))
            continue
        saves = collections.defaultdict(list)
        for entry in group:
            saves[entry["save"]].append(entry)
        ordered = sorted(saves, key=lambda key: hashlib.sha256(key.encode()).hexdigest())
        # Longest/heaviest campaign trace first; subsequent save representatives
        # alternate heavy and median compressed size. Size is only a proxy for
        # event richness, never a claim of measured event/mission coverage.
        queue = [max(group, key=lambda e: (e["frames"], e["bytes"], e["path"]))]
        for index, save in enumerate(ordered):
            variants = sorted(saves[save], key=lambda e: (e["bytes"], e["path"]))
            queue.append(variants[-1] if index % 2 == 0 else variants[len(variants)//2])
        queue.extend(sorted(group, key=lambda e: hashlib.sha256(e["path"].encode()).hexdigest()))
        queues[campaign] = queue
    selected, seen = [], set()
    # Always cover all interactive and very small replacement campaigns.
    for campaign, group in sorted(groups.items()):
        if campaign == "interactive" or len(group) <= 3:
            for entry in queues[campaign]:
                selected.append(entry)
                seen.add(entry["path"])
    while len(selected) < count:
        progressed = False
        for campaign, queue in queues.items():
            while queue and queue[0]["path"] in seen:
                queue.pop(0)
            if queue and len(selected) < count:
                entry = queue.pop(0)
                selected.append(entry)
                seen.add(entry["path"])
                progressed = True
        if not progressed:
            break
    selected = selected[:count]
    lanes = collections.defaultdict(collections.deque)
    for entry in selected:
        lanes[entry["campaign"]].append(entry)
    ordered = []
    while any(lanes.values()):
        for campaign in sorted(lanes):
            if lanes[campaign]:
                ordered.append(lanes[campaign].popleft())
    return ordered


def plan(args):
    corpus = args.corpus.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=False)
    entries, summary = inventory(corpus)
    selected = choose(entries, args.count)
    for entry in selected:
        entry["sha256"] = digest(corpus / entry["path"])
    manifest = dict(manifest_version=1, source_commit=args.commit,
                    source_corpus=str(corpus), selection_version=1,
                    selection="all interactive/rare groups, campaign round-robin, stable save hash, heavy/median size",
                    coverage_limit="Save/campaign diversity and size are proxies; distinct mission IDs and event types are not decoded by this inventory.",
                    inventory=summary, artifacts=selected)
    write_json(args.output / "manifest.json", manifest)
    write_json(args.output / "inventory.json", dict(summary=summary, artifacts=entries))
    print(json.dumps(dict(inventory=summary, selected=len(selected),
                         selected_frames=sum(e["frames"] for e in selected),
                         selected_saves=len({e["save"] for e in selected}),
                         selected_campaigns=dict(collections.Counter(e["campaign"] for e in selected))), indent=2))


def core_identity(args):
    root = (getattr(args, "core_datadir", None) or Path(__file__).resolve().parents[2] / "assets/core-datadir").resolve(strict=True)
    return dict(path=str(root), audio_durations_sha256=digest(root / "Data/AudioDurations.json"))


def check_result_identity(result, trace, entry, runner_sha):
    """Both early divergence and EOF must belong to the selected inputs."""
    if (Path(result["trace_path"]).resolve() != trace.resolve()
            or result["native_trace_sha256"] != entry["sha256"]
            or result["executable_sha256"] != runner_sha):
        raise ValueError("structured result identity mismatch")


def run_case(entry, index, output, runner, runner_sha, datadir, timeout, core_datadir=None):
    core_datadir = (core_datadir or Path(__file__).resolve().parents[2] / "assets/core-datadir").resolve()
    started = time.monotonic()
    trace = output / "traces" / entry["path"]
    log_path = output / "logs" / f"{index:04d}.log"
    status, result, classification, error = None, None, "error", None
    error_kind = None
    lock_path = output / "locks" / (hashlib.sha256(entry["path"].encode()).hexdigest() + ".lock")
    with lock_path.open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            if digest(trace) != entry["sha256"]:
                raise ValueError("copied trace identity changed")
            with log_path.open("wb") as log:
                try:
                    status = subprocess.run([str(runner), "--no-auto-dump", "--core-datadir", str(core_datadir), str(trace)],
                        env=dict(os.environ, ROBINHOOD_DATA_DIR=str(datadir)),
                        stdout=log, stderr=subprocess.STDOUT, timeout=timeout, check=False).returncode
                except subprocess.TimeoutExpired:
                    classification, status = "timeout", 124
            log = log_path.read_text(errors="replace")
            result = read_result(log)
            if classification != "timeout":
                if result is not None:
                    check_result_identity(result, trace, entry, runner_sha)
                if result and result["outcome"] == "divergence":
                    classification = "divergence"
                elif status == 0 and exact_eof(log, trace=trace):
                    if (result["processed_frames"] != entry["frames"]
                            or result["final_frame"] != entry["final_frame"]):
                        raise ValueError("structured EOF extent mismatch")
                    classification = "exact_eof"
                else:
                    error = "runner did not produce an admitted exact EOF result"
                    if "unsupported" in log.lower() and "schema" in log.lower():
                        error_kind = "unsupported_schema"
                    elif "chdir to ROBINHOOD_DATA_DIR" in log or "No such file or directory" in log:
                        error_kind = "setup_or_missing_file"
                    elif "decode" in log.lower() and "panicked" in log.lower():
                        error_kind = "decode_or_reconstruction"
                    else:
                        error_kind = "runner_error_or_incomplete"
        except (OSError, ValueError) as exception:
            error = str(exception)
            error_kind = "evidence_or_io_error"
    record = dict(index=index, trace=entry["path"], campaign=entry["campaign"],
                  classification=classification, status=status, error=error, error_kind=error_kind,
                  elapsed_seconds=time.monotonic()-started,
                  log=str(log_path), log_sha256=digest(log_path) if log_path.exists() else None,
                  result=result)
    write_json(output / "results" / f"{index:04d}.json", record)
    return record


def recover(output, manifest, args):
    """Verify durable evidence before resuming; never replace completed results."""
    launch = json.loads((output / "launch.json").read_text())
    runner = output / "original_parity_replay"
    validator = Path(__file__).resolve().parents[1] / "parity_result.py"
    if (digest(output / "manifest.json") != launch["manifest_sha256"]
            or digest(runner) != launch["runner_sha256"]
            or digest(args.runner) != launch["runner_sha256"]
            or digest(validator) != launch["validator_sha256"]
            or manifest["source_commit"] != launch["source_commit"]
            or str(args.datadir.resolve(strict=True)) != launch["datadir"]
            or core_identity(args) != launch.get("core_input")
            or (args.workers, args.timeout, args.hours) !=
               (launch["workers"], launch["timeout_seconds"], launch["hours"])):
        raise ValueError("resume provenance/configuration mismatch")
    if (output / "campaign-result.json").exists():
        raise ValueError("campaign already finalized")
    entries = manifest["artifacts"]
    for entry in entries:
        relative = Path(entry["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("unsafe manifest trace path")
        if digest(output / "traces" / relative) != entry["sha256"]:
            raise ValueError(f"copied trace changed: {relative}")
    db = sqlite3.connect(output / "ledger.sqlite3")
    rows = list(db.execute("SELECT id, trace, state, result_json FROM cases ORDER BY id"))
    db.close()
    if [(r[0], r[1]) for r in rows] != list(enumerate(e["path"] for e in entries)):
        raise ValueError("ledger/manifest mismatch")
    records, interrupted, todo = [], [], []
    for index, trace, state, encoded in rows:
        result_path = output / "results" / f"{index:04d}.json"
        if result_path.exists():
            record = json.loads(result_path.read_text())
            log_path = output / "logs" / f"{index:04d}.log"
            if (record["index"] != index or record["trace"] != trace
                    or record["log"] != str(log_path)
                    or record["log_sha256"] != digest(log_path)
                    or (encoded is not None and json.loads(encoded) != record)
                    or record["classification"] not in ("exact_eof", "divergence", "error", "timeout")
                    or (encoded is not None and state != record["classification"])):
                raise ValueError(f"saved result mismatch: {index}")
            if record["classification"] in ("exact_eof", "divergence"):
                log = log_path.read_text(errors="replace")
                result = read_result(log)
                entry = entries[index]
                if (result is None or result != record["result"]
                        or result["outcome"] != record["classification"]):
                    raise ValueError(f"saved structured result mismatch: {index}")
                check_result_identity(result, output / "traces" / trace, entry, launch["runner_sha256"])
                if record["classification"] == "exact_eof" and (
                        record["status"] != 0 or not exact_eof(log, trace=output / "traces" / trace)
                        or result["processed_frames"] != entry["frames"]
                        or result["final_frame"] != entry["final_frame"]):
                    raise ValueError(f"saved EOF mismatch: {index}")
            records.append(record)
        elif state in ("pending", "running") and encoded is None:
            todo.append(index)
            if state == "running":
                interrupted.append(index)
        else:
            raise ValueError(f"missing completed evidence: {index}")
    # All checks precede mutations. Archive partial logs by rename so stale open
    # descriptors cannot write into a new attempt's log. Cause is not inferred.
    archive = output / "interruptions" / str(time.time_ns())
    archive.mkdir(parents=True)
    for index in interrupted:
        log = output / "logs" / f"{index:04d}.log"
        if log.exists():
            log.rename(archive / log.name)
    write_json(archive / "resume.json", dict(pid=os.getpid(), command=sys.argv,
        resumed_unix=time.time(), original_started_unix=launch["started_unix"],
        deadline_unix=launch["started_unix"] + launch["hours"] * 3600,
        interrupted_cases=interrupted, interruption="external_termination_cause_unknown",
        preserved_results=len(records), script_sha256=digest(__file__),
        runner_sha256=launch["runner_sha256"], manifest_sha256=launch["manifest_sha256"]))
    db = sqlite3.connect(output / "ledger.sqlite3")
    for record in records:
        db.execute("UPDATE cases SET state=?, result_json=? WHERE id=?",
                   (record["classification"], json.dumps(record), record["index"]))
    db.execute("UPDATE cases SET state='pending' WHERE state='running'")
    db.commit()
    return db, records, todo, launch


def run(args):
    output = args.output.resolve(strict=True)
    manifest_path = output / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("manifest_version") != 1:
        raise ValueError("unsupported manifest")
    corpus = Path(manifest["source_corpus"])
    entries = manifest["artifacts"]
    with (output / "campaign.lock").open("w") as campaign_lock:
        fcntl.flock(campaign_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if getattr(args, "resume", False):
            db, records, queue, launch = recover(output, manifest, args)
            try:
                return dispatch(args, output, manifest, db, records, queue, launch)
            finally:
                db.close()
        for name in ("logs", "locks", "results", "traces"):
            (output / name).mkdir(exist_ok=False)
        runner = output / "original_parity_replay"
        runner_sha = digest(args.runner)
        shutil.copy2(args.runner, runner)
        if digest(runner) != runner_sha:
            raise ValueError("runner changed while copying")
        write_json(output / "launch.json", dict(pid=os.getpid(), command=sys.argv,
            source_commit=manifest["source_commit"], source_runner=str(args.runner.resolve()),
            runner_sha256=runner_sha, manifest_sha256=digest(manifest_path),
            script_sha256=digest(__file__), validator_sha256=digest(Path(__file__).resolve().parents[1]/"parity_result.py"),
            workers=args.workers, timeout_seconds=args.timeout, hours=args.hours,
            datadir=str(args.datadir.resolve(strict=True)), core_input=core_identity(args), started_unix=time.time()))
        for entry in entries:
            relative = Path(entry["path"])
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("unsafe manifest trace path")
            source, target = corpus / relative, output / "traces" / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
            if digest(target) != entry["sha256"]:
                raise ValueError(f"trace changed before copy: {relative}")
        db = sqlite3.connect(output / "ledger.sqlite3")
        db.execute("CREATE TABLE cases (id INTEGER PRIMARY KEY, trace TEXT UNIQUE NOT NULL, state TEXT NOT NULL, result_json TEXT)")
        db.executemany("INSERT INTO cases VALUES (?, ?, 'pending', NULL)", enumerate(e["path"] for e in entries))
        db.commit()
        launch = json.loads((output / "launch.json").read_text())
        try:
            return dispatch(args, output, manifest, db, [], list(range(len(entries))), launch)
        finally:
            db.close()


def dispatch(args, output, manifest, db, records, queue, launch):
    entries = manifest["artifacts"]
    manifest_path = output / "manifest.json"
    runner = output / "original_parity_replay"
    runner_sha = launch["runner_sha256"]
    started = time.monotonic() - (time.time() - launch["started_unix"])
    deadline = started + launch["hours"] * 3600
    next_index = 0
    write_json(output / "progress.json", dict(completed=len(records), total=len(entries),
        counts=dict(collections.Counter(r["classification"] for r in records)),
        running=[], elapsed_seconds=time.monotonic()-started))
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        pending = {}
        while pending or next_index < len(queue):
            while len(pending) < args.workers and next_index < len(queue) and time.monotonic() < deadline:
                if core_identity(args) != launch["core_input"]:
                    raise ValueError("core timing input changed during sweep")
                index = queue[next_index]
                next_index += 1
                db.execute("UPDATE cases SET state='running' WHERE id=?", (index,))
                db.commit()
                timeout = min(args.timeout, max(0.01, deadline-time.monotonic()))
                future = pool.submit(run_case, entries[index], index, output, runner,
                                     runner_sha, args.datadir.resolve(strict=True), timeout,
                                     Path(launch["core_input"]["path"]))
                pending[future] = index
                print(f"START {index:04d} {entries[index]['path']}", flush=True)
            if not pending:
                break
            done, _ = concurrent.futures.wait(pending, timeout=30,
                return_when=concurrent.futures.FIRST_COMPLETED)
            for future in done:
                index = pending.pop(future)
                record = future.result()
                if core_identity(args) != launch["core_input"]:
                    raise ValueError("core timing input changed during sweep")
                records.append(record)
                db.execute("UPDATE cases SET state=?, result_json=? WHERE id=?",
                           (record["classification"], json.dumps(record), index))
                db.commit()
                counts = dict(collections.Counter(r["classification"] for r in records))
                write_json(output / "progress.json", dict(completed=len(records), total=len(entries),
                           counts=counts, running=list(pending.values()), elapsed_seconds=time.monotonic()-started))
                print(f"DONE {index:04d} {record['classification']} status={record['status']} counts={counts}", flush=True)
            write_json(output / "progress.json", dict(completed=len(records), total=len(entries),
                counts=dict(collections.Counter(r["classification"] for r in records)),
                running=list(pending.values()), elapsed_seconds=time.monotonic()-started))
    if core_identity(args) != launch["core_input"]:
        raise ValueError("core timing input changed during sweep")
    db.execute("UPDATE cases SET state='not_run_budget' WHERE state='pending'")
    db.commit()
    counts = dict(db.execute("SELECT state, count(*) FROM cases GROUP BY state"))
    report = dict(campaign_version=1, source_commit=manifest["source_commit"],
                  runner_sha256=runner_sha, manifest_sha256=digest(manifest_path),
                  selected=len(entries), workers=args.workers, timeout_seconds=args.timeout,
                  budget_hours=args.hours, elapsed_seconds=time.monotonic()-started,
                  counts=counts, results=sorted(records, key=lambda r:r["index"]))
    write_json(output / "campaign-result.json", report)
    db.close()
    print(json.dumps(dict(final_counts=counts, report=str(output / "campaign-result.json"))), flush=True)
    return 0 if counts == {"exact_eof": len(entries)} else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("plan")
    p.add_argument("--corpus", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--commit", required=True)
    p.add_argument("--count", type=int, default=256)
    p = commands.add_parser("run")
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--runner", type=Path, required=True)
    p.add_argument("--datadir", type=Path, required=True)
    p.add_argument("--core-datadir", type=Path, default=Path(__file__).resolve().parents[2] / "assets/core-datadir")
    p.add_argument("--workers", type=int, choices=(1,2), default=2)
    p.add_argument("--timeout", type=int, default=1800)
    p.add_argument("--hours", type=float, default=8)
    p.add_argument("--resume", action="store_true", help="resume verified evidence without resetting original budget")
    args = parser.parse_args()
    if args.command == "plan":
        if args.count < 1:
            parser.error("count must be positive")
        plan(args)
    else:
        if not 0 < args.hours <= 8 or args.timeout <= 0:
            parser.error("budget must be in (0,8] hours and timeout must be positive")
        raise SystemExit(run(args))


if __name__ == "__main__":
    main()
