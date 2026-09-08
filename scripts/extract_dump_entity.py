#!/usr/bin/env python3
"""Extract per-frame state for one entity from a --dump-jsonl window."""
import json, sys

path = sys.argv[1]
want_kind = sys.argv[2]  # soldier/pc/civilian/projectile
want_idx = int(sys.argv[3])
fields = sys.argv[4].split(",") if len(sys.argv) > 4 else ["direction_goal", "direction", "animation", "command", "motion_state", "position_map"]

for line in open(path):
    o = json.loads(line)
    if o.get("type") == "header":
        continue
    engine = o.get("engine", {})
    frame = engine.get("frame", o.get("frame", "?"))
    # find entity in elements
    elements = engine.get("elements", o.get("elements", []))
    if isinstance(elements, dict):
        elements = [elements]
    for e in elements or []:
        if e.get("kind") == want_kind and e.get("index") == want_idx:
            ed = e.get("element", e)
            vals = {}
            for f in fields:
                if f in ed:
                    vals[f] = ed[f]
            print(f"frame={frame} {vals}")
