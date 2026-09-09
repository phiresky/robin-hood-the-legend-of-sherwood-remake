#!/usr/bin/env python3
"""Build a browser regression replay from 75 real, deterministic demo ticks.

Input is a JSONL recording or the root chunk of a mission archive. No game
assets or save payloads are embedded. Console outcomes deliberately make this
playback fixture unranked; ranked restore verification has separate engine tests.
"""
import argparse
import copy
import json
from pathlib import Path


def history_fixture(lines):
    header = lines[0].get("recording", lines[0])
    frames = {line["f"]: line for line in lines[1:] if "i" in line}
    for ordinal in range(76):
        if ordinal not in frames:
            raise ValueError("fixture requires a complete 75-tick recording")
    for ordinal in (50, 75):
        if "h" not in frames[ordinal]:
            raise ValueError(f"fixture requires recorded checkpoint {ordinal}")
    output = [copy.deepcopy(header)]
    output[0]["total_frames"] = 0
    output.extend(copy.deepcopy(line) for line in lines[1:] if line["f"] <= 75)
    for checkpoint in (50, 75):
        output.append({"f": checkpoint, "sv": {
            "state_hash": frames[checkpoint]["h"],
            "timeline_frame": frames[checkpoint]["i"]["timeline_before"],
        }})
    next_ordinal = 76

    def boundary(timeline):
        nonlocal next_ordinal
        record = copy.deepcopy(frames[1])
        record.pop("h", None)
        record["f"] = next_ordinal
        next_ordinal += 1
        record["i"].update(timeline_before=timeline, timeline_after=timeline)
        record["i"]["host_controls"] = []
        record["i"]["input"].update(commands=[], post_commands=[], external_actions=[],
            post_external_actions=[], run_hourglass=False, simulation_body_allowed=False,
            run_post_initialize=False)
        output.append(record)
        return record

    def terminal(timeline, won):
        record = boundary(timeline)
        record["i"]["timeline_after"] = timeline + 1
        inputs = record["i"]["input"]
        inputs.update(run_hourglass=True, simulation_body_allowed=True)
        inputs["external_actions"] = [{"kind": "console_command",
            "command": "WinMission" if won else "LoseMission", "selected_view_element": None}]
        if won:
            inputs["commands"] = [{"player_id": 0, "command": "QuitMissionRequested"}]
        inputs["post_commands"] = [{"player_id": 0, "command": {"ApplyQuitMissionUpdates": {
            "exit_code": "LevelSucceeded" if won else "LevelFailed",
            "difficulty": header["sim_config"]["difficulty"],
            "completed_at_unix_seconds": None, "campaign_run_nonce": 1,
        }}}]

    def restore(target, checkpoint):
        record = boundary(frames[checkpoint]["i"]["timeline_before"])
        record["lb"] = {"to_frame": target, "is_continue": False, "snapshot": None}
        # Loading reconciles persisted state; pre-save hashes are not
        # post-load checkpoints. Retain only original-prefix hashes.

    def repeat(start, end):
        nonlocal next_ordinal
        for ordinal in range(start, end):
            record = copy.deepcopy(frames[ordinal])
            record.pop("h", None)
            record["f"] = next_ordinal
            next_ordinal += 1
            output.append(record)

    terminal(frames[75]["i"]["timeline_after"], False)
    restore(50, 50)
    repeat(50, 75)
    terminal(frames[75]["i"]["timeline_before"], True)
    restore(75, 75)
    repeat(75, 76)
    restore(50, 50)
    repeat(50, 76)
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    lines = [json.loads(line) for line in args.input.read_text().splitlines() if line]
    output = history_fixture(lines)
    args.output.write_text("".join(json.dumps(line, separators=(",", ":")) + "\n" for line in output))
    print(f"Created {args.output}: two saves, loss, restore, win, restore, older restore, EOF")
