#!/usr/bin/env python3
"""Save/load scenario for frame_steps_live.py --save-load.

No process launch of its own: reuse the bounded, isolated exact-binary harness.
"""

import copy
import json
import os
from pathlib import Path
import re
import subprocess
import sys


def restored_authority(saved, restored):
    """Accept only the exact documented post-load reconciliation trailer.

    Engine::post_load_sync broadcasts posture/action and schedules HUD refresh.
    Do not discard whole queues: script subscribers can observe their contents.
    """
    result = copy.deepcopy(restored)
    seat = saved["players"]["seats"][0]
    if seat["selected_action"] != "NoAction" or not seat["selection"]:
        raise AssertionError("save/load fixture requires a selected PC and NoAction")
    queue = saved["orders"]["messenger"]["queue"]
    trailer = [
        {"arg1": 0, "arg2": 0, "msg_type": {"Simple": "Stature"}, "value": 0},
        {"arg1": 0, "arg2": 0, "msg_type": {"Pc": ["SelectAction", seat["selection"][0]]}, "value": 0},
    ]
    if result["orders"]["messenger"]["queue"] != queue + trailer:
        raise AssertionError("unexpected post-load messenger reconciliation")
    result["orders"]["messenger"]["queue"] = copy.deepcopy(queue)
    effects = saved["scripts"]["mission"]["script_effects"]["ordered"]
    if result["scripts"]["mission"]["script_effects"]["ordered"] != effects + [{"Presentation": "UpdateInformationBars"}]:
        raise AssertionError("unexpected post-load presentation reconciliation")
    result["scripts"]["mission"]["script_effects"]["ordered"] = copy.deepcopy(effects)
    return result


def verify_restored(saved, restored, saved_frame, restored_frame):
    if restored_frame != saved_frame:
        raise AssertionError(f"load restored frame {restored_frame}, expected {saved_frame}")
    if saved != restored:
        raise AssertionError("load-back authoritative engine state differs from saved state")


def verify_replay(log, load_record_frame, final_record_frame):
    if "Replay desync" in log:
        raise AssertionError("save/load recording replay desynchronized")
    hashes = [int(frame) for frame in re.findall(r"Replay hash OK @ frame (\d+)", log)]
    if not any(load_record_frame + 25 <= frame <= final_record_frame for frame in hashes):
        raise AssertionError("no verified replay hash beyond the restored frame")
    if "headless replay finished" not in log and "Replay finished after" not in log:
        raise AssertionError("save/load replay did not finish")


def recording_bounds(directory):
    """Read the two chronological chunks created by this one-load scenario."""
    manifest = json.loads((directory / "mission.json").read_text())
    chunks = manifest["chunks"]
    if manifest["version"] != 1 or len(chunks) != 2:
        raise AssertionError("expected a version-1 recording with exactly two chunks")
    records = []
    previous = None
    for index, chunk in enumerate(chunks):
        if (chunk["file"] != f"{index:08d}.rhrec.jsonl"
                or chunk["previous"] != previous
                or (chunk["loaded_save"] is not None) != (index == 1)):
            raise AssertionError("unexpected save/load recording chunk linkage")
        rows = [json.loads(line) for line in (directory / chunk["file"]).read_text().splitlines()]
        if rows[0]["chunk"] != chunk:
            raise AssertionError("recording header differs from manifest")
        records.extend(rows[1:])
        previous = chunk["file"]
    loads = [record["f"] for record in records if any(item.get("kind") == "state_load" for item in record.get("t", []))]
    if loads != [chunks[1]["first_ordinal"]]:
        raise AssertionError(f"expected one state_load at the second chunk boundary, observed {loads}")
    return loads[0], max(record.get("f", 0) for record in records)


def exercise_save_load(request, wait, display, evidence, summary):
    def key(name):
        subprocess.run([sys.executable, str(Path(__file__).with_name("client_x11.py")), "key", name],
                       env={**os.environ, "DISPLAY": display}, check=True, timeout=10)

    saved_frame = request("/state")["frame"]
    saved = request("/engine-dump")
    (evidence / "saved.engine.json").write_text(json.dumps(saved))
    # Fresh profiles select the original key preset: F1 save / F5 load.
    key("F1")
    save = wait("native QuickSave persisted", lambda: next((evidence / "live/save").rglob("QuickSave.json"), None))
    summary["save_load"] = {"saved_frame": saved_frame, "save_file": str(save)}
    advanced = request("/step-forward", {"n": 75, "auto_dismiss": True})
    if advanced["advanced"] != 75:
        raise AssertionError(f"failed to advance away from save: {advanced}")
    away = request("/state")["frame"]
    if away != saved_frame + 75:
        raise AssertionError(f"unexpected pre-load frame: {away}")
    key("F5")
    wait("native quickload restored frame", lambda: request("/state")["frame"] == saved_frame)
    restored = request("/engine-dump")
    (evidence / "restored.engine.json").write_text(json.dumps(restored))
    verify_restored(saved, restored_authority(saved, restored), saved_frame, request("/state")["frame"])
    summary["checks"]["native_save_load_restored_state"] = True
    continued = request("/step-forward", {"n": 100, "auto_dismiss": True})
    if continued["advanced"] != 100:
        raise AssertionError(f"failed to continue after load: {continued}")
    summary["save_load"]["continued_frame"] = request("/state")["frame"]
    if summary["save_load"]["continued_frame"] != saved_frame + 100:
        raise AssertionError("continued recording did not advance exactly from the restored frame")
    load_frame, final_frame = recording_bounds(evidence / "live.rhrec.jsonl")
    summary["save_load"]["load_record_frame"] = load_frame
    summary["save_load"]["final_record_frame"] = final_frame
    summary["checks"]["recording_continues_after_state_load"] = True
