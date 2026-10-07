"""Measure failed native lift contacts without changing gameplay geometry.

Run with: uv run --with shapely python refinement/audit-lift-support.py REPORT
The report must include bound physical navigation and initial motion states.
This is a polygon diagnostic, not a replacement for actor traversal tests.
"""

import argparse
import json
from pathlib import Path

from shapely.geometry import Polygon, box
from shapely.ops import unary_union


def polygon(points):
    result = Polygon(points)
    if not result.is_valid or result.is_empty:
        raise ValueError("Invalid bound navigation contour")
    return result


def active(states, layer, area, state):
    return states[layer][area] & state == state


def inspect(bound, states, half):
    definition = bound["definition"]
    floor = polygon(definition["boundary"])
    support = [floor]
    for landing in bound["landings"]:
        free = polygon(landing["boundary"])
        exclusions = [polygon(hole) for hole in landing["holes"]]
        exclusions.extend(
            polygon(obstacle["polygon"])
            for obstacle in landing["obstacles"]
            if active(states, landing["layer"], landing["area"], obstacle["state"])
        )
        free = free.difference(unary_union(exclusions))
        support.append(free)
    support = unary_union(support)
    blockers = unary_union([
        polygon(obstacle["polygon"])
        for obstacle in definition["obstacles"]
        if active(states, bound["layer"], bound["area"],
                  bound["obstacle_states"][obstacle["motion_obstacle"]])
    ])
    effective = [value - 1 for value in half]
    plane = definition["plane"] if not definition.get("floor_patches", []) else None

    def height(point):
        return plane[0] * point[0] + plane[1] * point[1] + plane[2]

    def contact(point):
        x, y = point[:2]
        footprint = box(x - effective[0], y - effective[1],
                        x + effective[0], y + effective[1])
        missing = footprint.difference(support)
        blocked = footprint.intersection(blockers)
        measured = {
            "world": point,
            "unsupported_area": missing.area,
            "blocked_area": blocked.area,
            "unsupported_bounds": list(missing.bounds) if not missing.is_empty else None,
            "blocked_bounds": list(blocked.bounds) if not blocked.is_empty else None,
        }
        if plane is not None:
            heights = [height(point) for point in footprint.exterior.coords]
            measured["extrapolated_plane_footprint_height_range"] = [min(heights), max(heights)]
        return measured

    return {
        "bound_landings": len(bound["landings"]),
        "flight_height_range": (
            [min(map(height, definition["boundary"])), max(map(height, definition["boundary"]))]
            if plane is not None else None
        ),
        "doors": [
            {key: contact(door[key]) for key in ["inside", "middle"]}
            for door in definition["doors"]
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--file", help="Inspect only this exported descriptor filename")
    parser.add_argument("--half", type=float, nargs=2, default=[6, 3],
                        help="Actor half-diagonal; defaults to the native audit actor's 6 by 3")
    args = parser.parse_args()
    if any(not 1 < value < float("inf") for value in args.half):
        parser.error("--half requires finite values greater than one")
    source = json.loads(args.report.read_text())
    results = []
    for result in source["results"]:
        if args.file and result["file"] != args.file:
            continue
        bounds = result["failed_physical_navigation_initial_state"]
        for sector, bound in bounds.items():
            results.append({
                "file": result["file"], "sector": sector,
                **inspect(bound, result["initial_motion_states"], args.half),
            })
    if not results:
        parser.error("No failed bound physical navigation matched the request")
    print(json.dumps({
        "scope": "initial-state-contact-area-diagnostic-not-traversal-certification",
        "source": str(args.report), "half_diagonal": args.half,
        "notes": [
            "Uses the normal one-unit movement inset; no footprint reduction is applied.",
            "Does not close sub-ULP contour cracks; tiny nonzero areas require precision review.",
            "Measures inside and middle contacts; does not certify a path between them.",
        ],
        "results": results,
    }, indent=2))


if __name__ == "__main__":
    main()
