"""Measure the same simulated Lincoln walk, with normal rollback enabled."""
import json
import time
import urllib.request

BASE = "http://127.0.0.1:17648"


def request(path, data=None):
    body = None if data is None else json.dumps(data).encode()
    req = urllib.request.Request(BASE + path, body, {"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as response:
        value = json.load(response)
    if "error" in value:
        raise RuntimeError(value)
    return value


initial = request("/engine-dump")
if initial["control"]["frame_counter"] != 0:
    raise RuntimeError("Benchmark requires a fresh --start-paused Lincoln mission on port 17648")
if "Pc" not in initial["world"]["entities"][126]:
    raise RuntimeError("Benchmark requires Lincoln H01_Lin_VL with Robin as PC 126")
request("/command", {"SetFogOfWar": {"enabled": True}})
request("/step-forward", {"n": 1})
for x in (2130, 1937, 2130, 1937):
    request("/command", {"GroupMove": {
        "actors": [{"Pc": 126}], "destination": {"x": x, "y": 1384},
        "running": False, "show_marker": False, "goal_override": None,
        "goal_sector_index_override": None, "door_route_override": None,
        "recorded_gate_routes": [], "recorded_failed_gate_routes": []}})
    start = time.perf_counter()
    result = request("/step-forward", {"n": 60})
    if result["advanced"] != 60:
        raise RuntimeError(f"Benchmark could not advance all 60 frames: {result}")
    elapsed = time.perf_counter() - start
    engine = request("/engine-dump")
    fog = engine["players"]["fog_of_war"]
    print(json.dumps({"seconds": elapsed, "advanced": result,
        "regions": {k: {"polygons": len(v["polygons"]),
            "vertices": sum(len(p["exterior"]) + sum(map(len, p["interiors"])) for p in v["polygons"])}
            for k, v in fog.items() if isinstance(v, dict) and "polygons" in v}}), flush=True)
