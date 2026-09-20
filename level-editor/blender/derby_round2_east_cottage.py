"""Reconstruct the east cottage's torn thatch and exposed timber roof.

The reviewed scenery silhouette contains openings between the rear rafters.
Those openings are geometry, not dark pixels painted on a complete roof.
Dimensions behind the source view remain conservative reconstruction choices.
"""
import math
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector

ASSET = "derby-lower-east-cottage"
TAG = "east_cottage_open_roof_v12"


def source_point(x, y, z):
    angle = math.radians(35)
    return Vector((x, -(y + z * math.cos(angle)) / math.sin(angle), z))


def source_y(point):
    return -point.y * math.sin(math.radians(35)) - point.z * math.cos(math.radians(35))


def upper_frame(build, west, apex, near, eave):
    """Reconstruct straight timber members from reviewed source centerlines.

    The authored mask is silhouette evidence, not a contour to extrude. Fitting
    its raster stair steps literally produces implausible notches when the
    roof slope is foreshortened; square timber sections remain coherent in 3D.
    """
    normal = (near - apex).cross(eave - apex).normalized()
    direction = Vector((0, -math.cos(math.radians(35)), math.sin(math.radians(35))))

    def point(x, y):
        ray = source_point(x, y, 0)
        return ray + direction * normal.dot(apex - ray) / normal.dot(direction)

    # Endpoints follow actual rafters and braces, not every opaque mask pixel.
    # They are all constrained to the same roof plane as the lower framing.
    members = ([
        ((895, 1833), (847, 1863), 5.0),
        ((891, 1838), (870, 1866), 3.3),
        ((886, 1844), (877, 1868), 3.2),
        ((894, 1834), (887, 1868), 4.1),
        ((857, 1857), (888, 1865), 2.8),
    ] if west else [
        ((895, 1833), (943, 1868), 5.0),
        ((899, 1838), (904, 1865), 3.8),
        ((912, 1848), (915, 1867), 3.3),
        ((927, 1859), (934, 1869), 3.2),
        ((901, 1857), (939, 1871), 2.8),
    ])
    for a, b, width in members:
        build.beam(point(*a), point(*b), width)


class Builder:
    def __init__(self):
        self.vertices = []
        self.faces = []
        self.smooth = []

    def closed(self, points, faces, smooth=False):
        offset = len(self.vertices)
        self.vertices.extend(points)
        self.faces.extend(tuple(offset + i for i in face) for face in faces)
        self.smooth.extend([smooth] * len(faces))

    def beam(self, a, b, width=2.4):
        axis = (b - a).normalized()
        side = axis.cross(Vector((0, 0, 1)))
        if side.length < .01:
            side = axis.cross(Vector((0, 1, 0)))
        side.normalize()
        other = axis.cross(side).normalized()
        ring = [side * x * width / 2 + other * y * width / 2
                for x, y in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
        self.closed([p + d for p in (a, b) for d in ring],
                    [(3, 2, 1, 0), (4, 5, 6, 7), (0, 1, 5, 4),
                     (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)])

    def slab(self, points, thickness):
        n = len(points)
        self.closed(points + [p - Vector((0, 0, thickness)) for p in points],
                    [tuple(range(n)), tuple(reversed(range(n, n * 2)))] +
                    [(i, (i + 1) % n, (i + 1) % n + n, i + n) for i in range(n)])

    def finish(self, obj):
        mesh = bpy.data.meshes.new(obj.name + " / open timber and thick thatch")
        inverse = obj.matrix_world.inverted()
        mesh.from_pydata([inverse @ p for p in self.vertices], [], self.faces)
        mesh.update()
        for polygon, smooth in zip(mesh.polygons, self.smooth):
            polygon.use_smooth = smooth
        material = bpy.data.materials.get("East cottage reconstruction neutral")
        if material is None:
            material = bpy.data.materials.new("East cottage reconstruction neutral")
            material.diffuse_color = (.34, .31, .27, 1)
        mesh.materials.append(material)
        mesh.uv_layers.new(name="UVMap")
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bmesh.ops.triangulate(bm, faces=list(bm.faces))
        bmesh.ops.dissolve_degenerate(bm, edges=list(bm.edges), dist=.0001)
        defects = {"nonmanifold_edges": sum(not e.is_manifold for e in bm.edges),
                   "degenerate_faces": sum(f.calc_area() < 1e-6 for f in bm.faces)}
        if any(defects.values()):
            raise ValueError(defects)
        bm.to_mesh(mesh)
        bm.free()
        mesh.set_sharp_from_angle(angle=math.radians(40))
        obj.data = mesh
        obj[TAG] = True
        obj["round2_mask_evidence"] = (
            "Derby layer 0 mask 1; silhouette and rear rafter openings"
            if obj["source_node"] != "building-052" else
            "Source artwork identifies open yard trough; no reviewed mask assignment")
        return {"source_node": obj["source_node"], "vertices": len(mesh.vertices),
                "faces": len(mesh.polygons), **defects}


def refine(mask_path=None):
    bpy.context.view_layer.update()
    mask_path = Path(mask_path) if mask_path else Path(__file__).resolve().parents[2] / "datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.d/masks/000001.png"
    if not mask_path.is_file():
        raise FileNotFoundError(mask_path)
    targets = {o.get("source_node"): o for o in bpy.data.collections["Derby Working"].all_objects
               if o.type == "MESH" and not o.hide_render and o.get("asset_group") == ASSET}
    if set(targets) != {"building-049", "building-050", "building-052"}:
        raise ValueError("Unexpected east cottage ownership")
    if all(targets[n].get(TAG) for n in targets):
        return {"status": "existing"}

    # Front overhang and rear framing anchors were checked against source art
    # and the authored scenery silhouette, independently of prior roof prisms.
    left_front = source_point(815, 1941, 77)
    right_front = source_point(928, 1958, 77)
    left_back = source_point(847, 1864, 86)
    right_back = source_point(945, 1870, 86)
    center_front = (left_front + right_front) / 2
    center_back = (left_back + right_back) / 2
    ridge_front_t = .46

    def center(t):
        p = center_front.lerp(center_back, t)
        p.z = 77 + 49 * min(1, t / ridge_front_t)
        return p

    report = []
    for node, edge_front, edge_back in (("building-049", left_front, left_back),
                                        ("building-050", right_front, right_back)):
        build = Builder()
        def roof(t, u):
            eave = edge_front.lerp(edge_back, t)
            ridge = center(t)
            p = ridge.lerp(eave, u)
            # A hip is the intersection of the front and side roof slopes.
            # Multiplying the two gradients would make a false central fold
            # reaching all the way down to the front lip.
            a, b = 1 - u, t / ridge_front_t
            soft_hip = max(0, (a + b - math.sqrt((a - b) ** 2 + .08 ** 2)) / 2)
            p.z = eave.z + (126 - eave.z) * soft_hip
            # A soft bulge and downturned outer lip give thatch real thickness.
            p.z += 3.2 * math.sin(math.pi * u) * math.sin(math.pi * min(1, t / .46) / 2)
            front_roll = max(0, 1 - t / .12) * (1 - u * u)
            p.y -= 8 * front_roll
            p.z -= 2 * front_roll
            if node == "building-049":
                p.x += 5 * math.sin(math.pi * min(1, t / .55)) * u * u
            return p

        def framing(a, b, width):
            ay, by = source_y(a), source_y(b)
            if max(ay, by) < 1865:
                return
            if ay < 1865:
                a = a.lerp(b, (1865 - ay) / (by - ay))
            elif by < 1865:
                b = b.lerp(a, (1865 - by) / (ay - by))
            build.beam(a, b, width)

        # A watertight grid shell, with a torn back edge instead of a complete
        # solid roof. End points differ by slope to avoid an invented neat cut.
        nu, nt = 14, 18
        points = []
        for i in range(nt + 1):
            for j in range(nu + 1):
                u = j / nu
                end = .57 - .075 * u + .016 * math.sin(u * math.pi * 5)
                points.append(roof(end * i / nt, u))
        count = len(points)
        vertices = points + [p - Vector((0, 0, 3.8)) for p in points]
        faces = []
        for i in range(nt):
            for j in range(nu):
                a = i * (nu + 1) + j
                f = (a, a + 1, a + nu + 2, a + nu + 1)
                faces.extend((f, tuple(k + count for k in reversed(f))))
        boundary = list(range(nu + 1))
        boundary += [i * (nu + 1) + nu for i in range(1, nt + 1)]
        boundary += [nt * (nu + 1) + j for j in range(nu - 1, -1, -1)]
        boundary += [i * (nu + 1) for i in range(nt - 1, 0, -1)]
        for a, b in zip(boundary, boundary[1:] + boundary[:1]):
            faces.append((a, b, b + count, a + count))
        build.closed(vertices, faces, smooth=True)

        # Actual exposed rafters, eave plate and intermediate longitudinal
        # purlin. Rear gable is open, with no hidden full triangular backing.
        for t in (.53, .72, .89, 1):
            framing(roof(t, 0), roof(t, 1), 2.8)
        framing(roof(.46, 1), roof(1, 1), 3.2)
        framing(roof(.49, .52), roof(1, .52), 2.1)
        if node == "building-049":
            framing(center(.44), center(1), 3)
        upper_frame(build, node == "building-049", center(1), center(.46), roof(1, 1))

        # Wall volume reaches into the roof underside at the eave. The front
        # wall is tucked behind the lip, not lowered away from its attachment.
        top = [roof(.065, .90 * j / 8) - Vector((0, 0, 1)) for j in range(9)]
        for i in range(1, 17):
            t = .065 + (.985 - .065) * i / 16
            p = roof(t, .90) - Vector((0, 0, 1))
            if t > .52:
                p.z = edge_front.lerp(edge_back, t).z + 1
            top.append(p)
        last = center(.985)
        last.z = top[-1].z
        top.append(last)
        # Keep the measured ground plan. The new thatch footprint is larger
        # because it overhangs; it must not silently enlarge the building walls.
        old_center_front = Vector((868.977, -3495.394, 0))
        old_center_back = Vector((896.846, -3372.999, 0))
        old_edge_front = Vector((821.053, -3484.339, 0)) if node == "building-049" else Vector((914.255, -3505.847, 0))
        old_edge_back = Vector((848.841, -3362.336, 0)) if node == "building-049" else Vector((942.205, -3383.059, 0))
        plan = [old_center_front.lerp(old_edge_front, j / 8) for j in range(9)]
        plan += [old_edge_front.lerp(old_edge_back, i / 16) for i in range(1, 17)]
        plan += [old_center_back]
        for i, (p, xy) in enumerate(zip(top, plan)):
            # Solve the roof's horizontal parameterization at each wall point.
            t, u = (.1, i / 8) if i < 9 else ((i - 8) / 16, 1)
            if i == len(plan) - 1:
                t, u = 1, 0
            for _ in range(8):
                q = roof(t, u)
                dt = (roof(t + .001, u) - q) / .001
                du = (roof(t, u + .001) - q) / .001
                determinant = dt.x * du.y - dt.y * du.x
                dx, dy = xy.x - q.x, xy.y - q.y
                t += (dx * du.y - dy * du.x) / determinant
                u += (dt.x * dy - dt.y * dx) / determinant
            p.x, p.y = xy.x, xy.y
            p.z = roof(t, u).z - 1 if t <= .52 else 86
        # Thin walls leave a genuine open attic behind the rafters. A solid
        # volume cap would incorrectly turn that opening into a flat roof.
        for a, b in zip(top[:-1], top[1:]):
            axis = b - a
            inward = Vector((-axis.y, axis.x, 0)).normalized() * 2.4
            center_xy = (center_front + center_back) / 2
            if inward.dot(center_xy - (a + b) / 2) < 0:
                inward.negate()
            aa, bb = Vector((a.x, a.y, 0)), Vector((b.x, b.y, 0))
            build.closed([a, b, bb, aa, a + inward, b + inward, bb + inward, aa + inward],
                         [(0, 1, 2, 3), (7, 6, 5, 4), (0, 4, 5, 1),
                          (1, 5, 6, 2), (2, 6, 7, 3), (3, 7, 4, 0)])
        floor = [Vector((p.x, p.y, 1)) for p in (top[0], top[8], top[-2], top[-1])]
        build.slab(floor, 1)
        for t in (.56, .77, .985):
            p = roof(t, .92)
            p.z = edge_front.lerp(edge_back, t).z
            build.beam(Vector((p.x, p.y, 0)), p, 2.6)
        if node == "building-050":
            # The wheel leaning against the front wall is visible in the raw
            # artwork. Give its rim, spokes and hub actual shallow relief.
            wheel = source_point(894, 1995, 17)
            horizontal = (old_edge_front - old_center_front).normalized()
            vertical = Vector((0, 0, 1))
            normal = horizontal.cross(vertical).normalized()
            ring = [wheel + 15.5 * (horizontal * math.cos(i * math.tau / 36)
                                   + vertical * math.sin(i * math.tau / 36)) for i in range(36)]
            for a, b in zip(ring, ring[1:] + ring[:1]):
                build.beam(a, b, 1.6)
            for i in range(0, 36, 3):
                build.beam(wheel, ring[i], .9)
            build.beam(wheel - normal, wheel + normal * 2.5, 3.2)
        report.append(build.finish(targets[node]))

    # The detached owned yard object is a trough: retain its footprint and
    # height, but make the plainly visible open center real geometry.
    trough = Builder()
    corners = [Vector(p) for p in ((931.499, -3308.229, 12.209),
                                   (974.924, -3277.617, 12.209),
                                   (965.536, -3264.354, 12.209),
                                   (922.278, -3295.209, 12.209))]
    middle = sum(corners, Vector()) / 4
    inner = [p + (middle - p).normalized() * 2 for p in corners]
    for i in range(4):
        j = (i + 1) % 4
        trough.slab([corners[i], corners[j], inner[j], inner[i]], 11.2)
    trough.slab([Vector((p.x, p.y, 2)) for p in inner], 1.8)
    report.append(trough.finish(targets["building-052"]))
    bpy.context.view_layer.update()
    return {"status": "created", "objects": report,
            "preserved": "stable source IDs 049, 050, 052; barrel 051 outside ownership",
            "assembly": "front walls sample roof underside with 2.8 units overlap; rear roof deliberately open"}
