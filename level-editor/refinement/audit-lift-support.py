"""Measure failed native lift contacts without changing gameplay geometry.

Run with: uv run --with shapely python refinement/audit-lift-support.py REPORT
The report must include bound physical navigation and initial motion states.
This is a polygon diagnostic, not a replacement for actor traversal tests.
"""

import argparse
import copy
import json
import math
from pathlib import Path

from shapely.geometry import LineString, MultiPoint, Point, Polygon, box
from shapely.ops import unary_union


def polygon(points):
    result = Polygon(points)
    if not result.is_valid or result.is_empty:
        raise ValueError("Invalid bound navigation contour")
    return result


def active(states, layer, area, state):
    return states[layer][area] & state == state


def clearance_frame(bound, mode):
    """Re-express one planar contact for comparison, never for asset publication."""
    if mode == "ground":
        return bound
    if mode not in ("surface", "screen"):
        raise ValueError(f"Unknown clearance frame: {mode}")
    if bound["definition"].get("floor_patches", []):
        raise ValueError("A single planar clearance frame cannot represent a piecewise floor")
    result = copy.deepcopy(bound)
    a, b, _ = result["definition"]["plane"]
    if mode == "surface":
        # Positive square root of I + gradient*gradient^T. This preserves
        # lengths on the 3D plane, including the uphill direction, without
        # arbitrarily choosing a smaller actor or a camera-dependent axis.
        k = 1 / (math.hypot(1, a, b) + 1)
        xx, xy, yx, yy = 1 + k*a*a, k*a*b, k*a*b, 1 + k*b*b
    else:
        xx, xy, yx, yy = 1, 0, -a, 1-b
    determinant = xx*yy - xy*yx
    if not math.isfinite(determinant) or abs(determinant) < 1e-8:
        raise ValueError("Clearance frame is degenerate or non-finite")
    origin = result["definition"]["doors"][0]["middle"][:2]

    def point(p):
        x, y = p[0]-origin[0], p[1]-origin[1]
        return [xx*x + xy*y, yx*x + yy*y] + p[2:]

    def ring(points):
        return [point(p) for p in points]

    def plane(coefficients):
        a, b, c = coefficients
        return [(a*yy-b*yx)/determinant, (-a*xy+b*xx)/determinant,
                a*origin[0] + b*origin[1] + c]

    definition = result["definition"]
    definition["plane"] = plane(definition["plane"])
    definition["boundary"] = ring(definition["boundary"])
    for obstacle in definition["obstacles"]:
        obstacle["polygon"] = ring(obstacle["polygon"])
    for landing in result["landings"]:
        landing["plane"] = plane(landing["plane"])
        landing["boundary"] = ring(landing["boundary"])
        landing["holes"] = [ring(hole) for hole in landing["holes"]]
        for obstacle in landing["obstacles"]:
            obstacle["polygon"] = ring(obstacle["polygon"])
    for door in definition["doors"]:
        for key in ["inside", "middle", "outside"]:
            door[key] = point(door[key])
    return result


def runtime_offsets(bound, half):
    x, y = [value - 1 for value in half]
    rectangle = [[-x, -y], [x, -y], [x, y], [-x, y]]
    if not bound.get("climbing", False):
        return rectangle
    definition = bound["definition"]
    planes = [patch["plane"] for patch in definition.get("floor_patches", [])] or [definition["plane"]]
    offsets = []
    for a, b, _ in planes:
        length = math.hypot(a, b)
        if not length:
            offsets.extend(rectangle)
            continue
        nx, ny = a/length, b/length
        contraction = 1 - 1/math.hypot(length, 1)
        for x, y in rectangle:
            along = (x*nx+y*ny)*contraction
            offsets.append([x-along*nx, y-along*ny])
    return list(MultiPoint(offsets).convex_hull.exterior.coords)


def inspect(bound, states, half, offsets=None):
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

    def footprint(point):
        x, y = point[:2]
        if offsets is not None:
            return polygon([[x+dx, y+dy] for dx, dy in offsets])
        return box(x - effective[0], y - effective[1],
                   x + effective[0], y + effective[1])

    def coverage(shape):
        missing = shape.difference(support)
        blocked = shape.intersection(blockers)
        return {
            "unsupported_area": missing.area,
            "blocked_area": blocked.area,
            "unsupported_bounds": list(missing.bounds) if not missing.is_empty else None,
            "blocked_bounds": list(blocked.bounds) if not blocked.is_empty else None,
        }

    def contact(point):
        shape = footprint(point)
        measured = {
            "contact_coordinates": point, **coverage(shape),
            "nearest_free_center_distance": min(
                (component.distance(Point(point[:2])) for component in components), default=None
            ),
        }
        if plane is not None:
            heights = [height(point) for point in shape.exterior.coords]
            measured["extrapolated_plane_footprint_height_range"] = [min(heights), max(heights)]
        return measured

    def polygons(geometry):
        if geometry.is_empty:
            return []
        if geometry.geom_type == "Polygon":
            return [geometry]
        return [part for part in geometry.geoms if part.geom_type == "Polygon"]

    def swept_edges(geometry):
        sweeps = []
        for part in polygons(geometry):
            for ring in [part.exterior, *part.interiors]:
                points = list(ring.coords)
                for first, second in zip(points, points[1:]):
                    sweeps.append(unary_union([footprint(first), footprint(second)]).convex_hull)
        return unary_union(sweeps)

    # Exact rectangular configuration space, without the runtime's f32 crack
    # normalization. Component distances expose both real gaps and small
    # representation errors; they never silently move the requested endpoints.
    centers = support.difference(swept_edges(support))
    centers = centers.difference(blockers.union(swept_edges(blockers))).intersection(floor)
    components = polygons(centers)
    direct_routes = []
    for index, first in enumerate(definition["doors"]):
        for other, second in enumerate(definition["doors"][index+1:], index+1):
            start, end = first["inside"], second["inside"]
            swept = unary_union([footprint(start), footprint(end)]).convex_hull
            center = LineString([start[:2], end[:2]])
            distances = [
                [component.distance(Point(start[:2])), component.distance(Point(end[:2]))]
                for component in components
            ]
            direct_routes.append({
                "doors": [index, other],
                "center_length_outside_flight": center.difference(floor).length,
                "closest_common_free_component_endpoint_distances": (
                    min(distances, key=max) if distances else None
                ),
                **coverage(swept),
            })
    return {
        "bound_landings": len(bound["landings"]),
        "free_center_components": len(components),
        "flight_height_range": (
            [min(map(height, definition["boundary"])), max(map(height, definition["boundary"]))]
            if plane is not None else None
        ),
        "doors": [
            {key: contact(door[key]) for key in ["inside", "middle"]}
            for door in definition["doors"]
        ],
        "direct_inside_routes": direct_routes,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--file", help="Inspect only this exported descriptor filename")
    parser.add_argument("--frame", choices=["runtime", "ground", "surface", "screen"], default="runtime",
                        help="Use saved runtime clearance, or compare hypothetical clearance frames")
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
            framed = bound if args.frame == "runtime" else clearance_frame(bound, args.frame)
            results.append({
                "file": result["file"], "sector": sector,
                **inspect(framed, result["initial_motion_states"], args.half,
                          runtime_offsets(bound, args.half) if args.frame == "runtime" else None),
            })
    if not results:
        parser.error("No failed bound physical navigation matched the request")
    print(json.dumps({
        "scope": "initial-state-contact-area-diagnostic-not-traversal-certification",
        "source": str(args.report), "half_diagonal": args.half, "clearance_frame": args.frame,
        "notes": [
            "Uses the normal one-unit movement inset; no footprint reduction is applied.",
            "Does not close sub-ULP contour cracks; tiny nonzero areas require precision review.",
            "Measures contacts and straight inside-to-inside sweeps; blocked sweeps may admit detours.",
            "Alternative frames are experiments, not runtime behavior; areas use the selected frame.",
        ],
        "results": results,
    }, indent=2))


if __name__ == "__main__":
    main()
