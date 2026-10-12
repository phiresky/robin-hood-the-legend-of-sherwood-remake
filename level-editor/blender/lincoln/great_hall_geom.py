"""Shared geometry helpers for the Lincoln great_hall lane recipes.

All coordinates here are *native* (x, y, z): the source camera projects a native point
to pixel (x, y - z). Blender world coordinates are X = x, Y = -y / sin35, Z = z / cos35.

The main tool is ``notched_wall``: a closed wall slab built from a straight footprint
(front edge p0->p1, back edge q0->q1) and a (t, z) elevation profile. The profile is the
wall's front elevation, where t in [0, 1] runs along the wall. Crenellated parapets, stepped
gable rakes and sloped ramp parapets are all profiles, so every merlon is real geometry
cut into one continuous wall body, not a detached box.

Traces are stored as original full-image pixels. ``pixel_to_tz`` converts a pixel on
the front face of a wall into (t, z). Pixel x is exactly native x, so t follows from x
and z = y_front(t) - pixel_y.
"""
import hashlib
import json
import math
from pathlib import Path

try:  # Blender-only modules; the pure-Python helpers (traces, profiles) work without them.
    import bmesh
    import bpy
    from mathutils import Vector
except ImportError:  # pragma: no cover - outside Blender
    bmesh = bpy = None

    class Vector(tuple):
        def __new__(cls, v):
            return tuple.__new__(cls, v)

SIN = math.sin(math.radians(35))
COS = math.cos(math.radians(35))
SOURCE = Path(__file__).resolve().parents[2] / 'work/needs-review/lincoln-refinement/source-states/covered.png'


def to_world(p):
    x, y, z = p
    return Vector((x, -y / SIN, z / COS))


def to_native(v):
    return (v.x, -v.y * SIN, v.z * COS)


def project(p):
    """Native point -> source pixel."""
    return (p[0], p[1] - p[2])


def lerp2(a, b, t):
    return (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)


def pixel_to_tz(p0, p1, px, py):
    """Pixel on the front face of the wall p0->p1 -> (t, native z)."""
    if abs(p1[0] - p0[0]) < 1e-6:
        raise ValueError('Wall front edge is edge-on in x; pixel x cannot fix t')
    t = (px - p0[0]) / (p1[0] - p0[0])
    y = p0[1] + (p1[1] - p0[1]) * t
    return t, y - py


def crenel_profile(ground, floor, merlons, t0=0.0, t1=1.0):
    """Closed (t, z) front-elevation polygon.

    ground: bottom z (float or callable of t).
    floor: top of the wall between merlons, a callable z(t) (notch floor / wall walk
    parapet top, may slope).
    merlons: list of (ta, tb, top) with top a float or callable z(t) (for sloped
    merlon tops), sorted and non-overlapping inside [t0, t1].
    """
    g = ground if callable(ground) else (lambda t, v=ground: v)
    top = []
    cur = t0
    for ta, tb, zt in merlons:
        if ta < cur - 1e-9 or tb <= ta:
            raise ValueError(f'Merlons overlap or are unsorted near t={ta}')
        ztf = zt if callable(zt) else (lambda t, v=zt: v)
        if ta > cur + 1e-9:
            top.append((cur, floor(cur)))
        top.append((ta, floor(ta)))
        top.append((ta, ztf(ta)))
        top.append((tb, ztf(tb)))
        top.append((tb, floor(tb)))
        cur = tb
    if cur < t1 - 1e-9:
        top.append((cur, floor(cur)))
        top.append((t1, floor(t1)))
    # Drop consecutive duplicates (merlon flush with an end).
    clean = []
    for p in top:
        if not clean or abs(clean[-1][0] - p[0]) > 1e-9 or abs(clean[-1][1] - p[1]) > 1e-6:
            clean.append(p)
    if clean[0][0] > t0 + 1e-9:
        clean.insert(0, (t0, floor(t0)))
    if any(z <= g(t) + 1e-6 for t, z in clean):
        raise ValueError('Profile top at or below ground')
    bottom = [(t1, g(t1)), (t0, g(t0))]
    # Counter-clockwise in (t, z): bottom left -> bottom right -> top right ... top left.
    return [(t0, g(t0)), (t1, g(t1))] + list(reversed(clean))


def notched_wall(p0, p1, q0, q1, profile):
    """Closed prism: front polygon at footprint front edge, back polygon at back edge.

    Returns (native vertex list, faces)."""
    n = len(profile)
    verts = []
    for t, z in profile:
        x, y = lerp2(p0, p1, t)
        verts.append((x, y, z))
    for t, z in profile:
        x, y = lerp2(q0, q1, t)
        verts.append((x, y, z))
    faces = [list(range(n)), list(range(2 * n - 1, n - 1, -1))]
    for i in range(n):
        j = (i + 1) % n
        faces.append([i, n + i, n + j, j])
    return verts, faces


def prism(poly, z0, z1):
    """Vertical prism over a native (x, y) polygon."""
    n = len(poly)
    verts = [(x, y, z0) for x, y in poly] + [(x, y, z1) for x, y in poly]
    faces = [list(range(n - 1, -1, -1)), list(range(n, 2 * n))]
    for i in range(n):
        j = (i + 1) % n
        faces.append([i, j, n + j, n + i])
    return verts, faces


def merge(parts):
    verts, faces = [], []
    for v, f in parts:
        off = len(verts)
        verts.extend(v)
        faces.extend([[k + off for k in face] for face in f])
    return verts, faces


def replace_mesh(obj, native_verts, faces, weld=0.05):
    """Replace obj's mesh data in place (object, name, custom properties, materials and
    parenting are kept). Checks closed/manifold/non-degenerate and returns stats."""
    # Walls joined end-to-end (mitred ring corners) produce coincident end faces; they are
    # interior, so drop both copies (by position) to keep the welded shell manifold.
    def key(face):
        return frozenset(tuple(round(c, 3) for c in native_verts[k]) for k in face)
    counts = {}
    for face in faces:
        counts[key(face)] = counts.get(key(face), 0) + 1
    faces = [face for face in faces if counts[key(face)] == 1]
    inv = obj.matrix_world.inverted()
    mesh = bpy.data.meshes.new(obj.data.name + ' refined')
    mesh.from_pydata([inv @ to_world(p) for p in native_verts], [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=weld)
    bmesh.ops.dissolve_degenerate(bm, dist=1e-4, edges=list(bm.edges))
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    stats = {'open_edges': sum(1 for e in bm.edges if not e.is_manifold),
             'degenerate_faces': sum(1 for f in bm.faces if f.calc_area() < 1e-7),
             'vertices': len(bm.verts), 'faces': len(bm.faces)}
    bm.to_mesh(mesh)
    bm.free()
    for m in obj.data.materials:
        mesh.materials.append(m)
    for layer in obj.data.uv_layers:
        mesh.uv_layers.new(name=layer.name)
    old = obj.data
    obj.data = mesh
    name = old.name
    if old.users == 0:
        bpy.data.meshes.remove(old)
    mesh.name = name
    return stats


def clip_above_plane(obj, point, normal):
    """Remove the part of obj on the +normal side of a native plane and cap the cut.
    point is a native point, normal a native-space direction (converted to world)."""
    mw = obj.matrix_world
    inv = mw.inverted()
    pw = to_world(point)
    # Convert the native normal to world: plane through native points is preserved by the
    # affine native->world map; build two in-plane directions and cross them.
    n = Vector(normal).normalized()
    a = Vector((1, 0, 0)) if abs(n.x) < 0.9 else Vector((0, 1, 0))
    u = n.cross(a).normalized()
    w = n.cross(u).normalized()
    pu = to_world(Vector(point) + u) - pw
    pv = to_world(Vector(point) + w) - pw
    nw = pu.cross(pv).normalized()
    if nw.dot(to_world(Vector(point) + n) - pw) < 0:
        nw = -nw
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=0.3)
    co = inv @ pw
    no = (mw.to_3x3().transposed() @ nw).normalized()
    res = bmesh.ops.bisect_plane(bm, geom=list(bm.verts) + list(bm.edges) + list(bm.faces),
                                 plane_co=co, plane_no=no, clear_outer=True, dist=1e-5)
    cut_edges = [e for e in res['geom_cut'] if isinstance(e, bmesh.types.BMEdge) and e.is_boundary]
    filled = bmesh.ops.holes_fill(bm, edges=cut_edges, sides=0)['faces'] if cut_edges else []
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(obj.data)
    bm.free()
    obj.data.update()
    return {'clip_caps': len(filled)}


def plane_from_points(a, b, c):
    a, b, c = Vector(a), Vector(b), Vector(c)
    n = (b - a).cross(c - a).normalized()
    return a, n


def load_trace(path):
    data = json.loads(Path(path).read_text())
    digest = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    if data.get('source_sha256') != digest:
        raise ValueError(f'{path}: trace bound to a different source image')
    return data


def draw_trace(trace, crop, scale, out, mesh_edges=None):
    """Save the unmarked crop beside a numbered trace (+ optional projected mesh edges)."""
    from PIL import Image, ImageDraw
    src = Image.open(SOURCE).convert('RGB').crop(crop)
    big = src.resize((src.width * scale, src.height * scale), Image.NEAREST)
    ann = big.copy()
    d = ImageDraw.Draw(ann)
    if mesh_edges:
        for a, b in mesh_edges:
            pa, pb = project(a), project(b)
            d.line([((pa[0] - crop[0]) * scale, (pa[1] - crop[1]) * scale),
                    ((pb[0] - crop[0]) * scale, (pb[1] - crop[1]) * scale)], fill=(0, 200, 255), width=1)
    for run in trace['runs']:
        pts = [((c['px'] - crop[0]) * scale, (c['py'] - crop[1]) * scale) for c in run['corners']]
        if len(pts) > 1:
            d.line(pts, fill=(255, 255, 0), width=1)
        for c, p in zip(run['corners'], pts):
            col = (255, 60, 60) if c.get('confidence') != 'inferred' else (255, 160, 0)
            d.ellipse([p[0] - 3, p[1] - 3, p[0] + 3, p[1] + 3], outline=col)
            d.text((p[0] + 4, p[1] - 10), str(c['id']), fill=col)
    sheet = Image.new('RGB', (big.width * 2, big.height))
    sheet.paste(big, (0, 0))
    sheet.paste(ann, (big.width, 0))
    sheet.save(out)
