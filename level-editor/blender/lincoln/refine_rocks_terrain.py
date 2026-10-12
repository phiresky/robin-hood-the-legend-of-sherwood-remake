"""Lincoln rocks/terrain lane geometry recipe (idempotent, per workspace).

Run from the repository root, one asset per process:

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
        --python level-editor/blender/lincoln/refine_rocks_terrain.py -- \
        --asset <asset-id> [--packet] [--analyse]

The recipe acquires a Lincoln render slot, opens the asset workspace's
``model.blend`` and rebuilds only the owned meshes from the frozen native
geometry in ``inventory/inventory.json``; running it twice gives identical
meshes. ``--packet`` then regenerates ``modified/`` with the frozen tooling.
``--analyse`` writes the per-node support/carve report without saving.

Geometry rules (native units; pixel = (x, y - z)):

* Plateau and terrace volumes keep their native top heights and outlines.
  They are terrain shared with every other lane.
* A rock rebuild never exceeds the native obstacle prism. Its base replaces the
  native z = 0 datum by the reviewed terrain surface below it (``support``) or
  by a documented ground height measured from the native silhouette foot.
  Parts buried inside a supporting terrain volume are removed.
* ``carve`` lowers columns whose source pixel lies outside the node's reviewed
  native silhouette, unless that pixel is covered by a reviewed foreground
  exclusion mask (hidden, therefore unknown). The result is a closed
  heightfield solid with softened shoulders.

Terrain source-domain assignments (``MASKS``) are explicit reviewed receiver
changes to the asset's working ``source-masks.json``; they only use native
terrain silhouettes and are written idempotently.
"""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
import numpy as np
from mathutils import Matrix, Vector
from mathutils.geometry import tessellate_polygon

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import rocks_terrain_volumes_core as core  # noqa: E402

REPO = HERE.parents[2]
ROOT = REPO / 'level-editor/work/needs-review/lincoln-refinement'
ASSETS = ROOT / 'round-1/assets'
INVENTORY = ROOT / 'inventory/inventory.json'
TOOLING = ROOT / 'tooling/e6b57cb851c7142b'
TAG = 'lincoln_rocks_terrain_recipe'
VERSION = 'rocks-terrain-v2'

# Terrain volumes whose native tops are the reviewed ground for rocks. Tops are
# never changed by this lane, so other lanes may rely on the same surfaces.
SUPPORT_NODES = [52, 53, 54, 55, 56, 57, 61, 62, 63, 64, 65, 66, 67, 68, 70]


def _normalise(spec):
    if 'raise_' in spec:
        spec['raise'] = spec.pop('raise_')
    return spec


def rock(**kw):
    spec = {'mode': 'carve', 'cell': 2.0, 'support': True, 'floor': 0.0,
            'round': 10.0, 'edge': 0.3, 'occluders': [], 'smooth': 4,
            'min_component': 6, 'dome': False}
    spec.update(kw)
    return _normalise(spec)


def trim(**kw):
    spec = {'mode': 'trim', 'cell': 2.0, 'support': True, 'floor': 0.0,
            'round': 0.0, 'occluders': [], 'smooth': 0, 'min_component': 6}
    spec.update(kw)
    return _normalise(spec)


KEEP = {'mode': 'keep'}

# Per node rebuild specification. Measured ground heights come from the
# silhouette-foot analysis in inspection/ground-contact.json of each asset.
NODES = {
    # Ground-level boulders beside the stream and on the western map edge.
    436: rock(round=8.0),
    454: {'mode': 'stump'},
    438: rock(), 439: rock(), 440: rock(), 441: rock(), 442: rock(),
    # Hillside boulders below the great hall's north-west corner. Their painted
    # feet sit at native z 0..17 on the lower slope; the keep plateau outline
    # is not used as support here (see review).
    447: rock(cell=1.5, support=False), 448: rock(cell=1.5, support=False),
    449: rock(support=False, floor=15.0), 450: rock(support=False),
    451: rock(cell=1.5, support=False),
    # South-eastern bank rocks. The outcrop's painted foot descends to native
    # z ~150 east of the bank plateau outline; the small pair sits on it.
    443: rock(cell=3.0, floor=150.0, round=10.0, unhide=True),
    444: rock(cell=3.0, floor=150.0, round=10.0, unhide=True),
    445: rock(cell=1.5, unhide=True), 446: rock(cell=1.5, unhide=True),
    # West tower hillside: a rock on the lower northern slope behind the west
    # tower hall roof and a boulder on the western grass slope. The two
    # non-solid sight volumes 455/456 stand west of the plateau outline on the
    # native z = 0 hillside and are kept (see review).
    452: rock(support=False, occluders=[291, 297]),
    453: rock(),
    # 455/456 recede wherever they would hide the painted rocks 452/453.
    455: None, 456: None,
    # South-western castle cliff rock faces (shared moat-bank envelope 265).
    # They tile one continuous painted cliff, so the authored facets are kept
    # and only the parts buried in the cliff-road and plateau volumes removed.
    423: trim(), 424: trim(), 425: trim(), 426: trim(), 431: trim(),
    427: trim(), 428: trim(), 429: trim(), 430: trim(),
    # Inner rock spur and raised upper-ward ground. The spur envelopes are
    # largely hidden behind the inner west gate, so columns are not carved.
    # 432/433 carry the reviewed outcrop envelope 267 and are carved to it (the
    # great hall west wing masonry shows above them); 434 is raised to the
    # painted crest traced at the keep annex round turret; 435/465 keep their
    # native tops above the bailey plateau.
    # Garden walls 212/213/220 and corner tower 322 stand on the spur: their
    # pixels hide rock (support it); unhide keeps the rock behind their faces.
    432: rock(cell=2.0, round=6.0, floor=220.0, occluders=[212, 213, 220, 322], unhide=True, unhide_margin=2),
    433: rock(cell=2.0, round=6.0, floor=220.0, occluders=[212, 213, 220, 322], unhide=True),
    434: trim(raise_={'points': [[1655, 1117, 1498], [1670, 1120, 1512], [1693, 1147, 1521],
                                  [1717, 1177, 1521], [1733, 1207, 1518]],
                       'slope': 1.6, 'reach': 45}, unhide=True, unhide_margin=2),
    435: trim(unhide=True, unhide_margin=2), 465: trim(),
}

# South-western ravine bridge (round-2 revision): the parapets 074/076 were
# datum pillars reaching z = 0 through the ravine; they now stand on the deck
# 056 (and the road volumes at their ends). Deck 056 and arch wall 075 keep
# their native volumes (the painted arch wall reaches the ravine floor).
BRIDGE_PARAPET = dict(support_nodes=[56, 57], cell=1.0, floor=150.0)
# Terrain assets keep their native geometry.
for _n in (52, 53, 54, 55, 56, 57, 61, 62, 63, 64, 65, 66, 67, 68, 74, 75, 76):
    NODES[_n] = KEEP
# Round 3: 071 (marker slab from the NE tower stair, now in the north-bailey
# plateau group) is a collision-only marker; it is sunk just below the
# plateau top so it never shows or receives source pixels.
NODES[71] = {'mode': 'bury', 'top': 140.0, 'depth': 3.0}
NODES[74] = trim(**BRIDGE_PARAPET)
NODES[76] = trim(**BRIDGE_PARAPET)

from rocks_terrain_volumes_masks import (MASKS, FOLIAGE, WALLS, CLIFF_WALLS,  # noqa: E402
                                          CLIFF_FOLIAGE)


def sha(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def node_name(n):
    return f'building-{n:03d}'


# --------------------------------------------------------------------------
# Native evidence
# --------------------------------------------------------------------------

_INVENTORY = None


def native_triangles(source_node):
    """Frozen native mesh of a node as native-coordinate triangles."""
    global _INVENTORY
    if _INVENTORY is None:
        _INVENTORY = {o['source_node']: o for o in json.loads(INVENTORY.read_text())['objects']}
    record = _INVENTORY[source_node]
    matrix = Matrix(record['matrix_world'])
    world = [matrix @ Vector(v) for v in record['geometry']['vertices']]
    tris = []
    for face in record['geometry']['faces']:
        pts = [world[i] for i in face]
        for a, b, c in tessellate_polygon([pts]):
            tris.append([pts[a], pts[b], pts[c]])
    tris = np.array([[list(p) for p in t] for t in tris], float)
    native = core.world_to_native(tris.reshape(-1, 3)).reshape(-1, 3, 3)
    return native, matrix


class MaskStore:
    """Native mask bitmaps (source-image sized booleans) from the working manifest."""

    def __init__(self, manifest_path):
        manifest = json.loads(Path(manifest_path).read_text())
        inventory_path = (Path(manifest_path).parent / manifest['mask_inventory']).resolve(strict=True)
        self.dir = inventory_path.parent
        inventory = json.loads(inventory_path.read_text())
        self.records = {r['index']: r for r in inventory['masks']}
        self.size = (2176, 2944)
        self.cache = {}

    def get(self, index):
        if index in self.cache:
            return self.cache[index]
        record = self.records[index]
        full = np.zeros(self.size, bool)
        if record.get('png') is None:
            raise ValueError(f'Degenerate mask {index}')
        image = bpy.data.images.load(str(self.dir / record['png']), check_existing=False)
        w, h = image.size
        px = np.empty(w * h * 4, np.float32)
        image.pixels.foreach_get(px)
        bpy.data.images.remove(image)
        # Blender stores rows bottom-up.
        a = px.reshape(h, w, 4)[::-1]
        on = (a[..., :3].max(-1) > 0.5) & (a[..., 3] > 0.5)
        x, y = record['box_top_left']
        y0, y1 = max(0, y), min(self.size[0], y + h)
        x0, x1 = max(0, x), min(self.size[1], x + w)
        full[y0:y1, x0:x1] = on[y0 - y:y1 - y, x0 - x:x1 - x]
        self.cache[index] = full
        return full

    def union(self, indices):
        out = np.zeros(self.size, bool)
        for i in indices:
            out |= self.get(i)
        return out


# --------------------------------------------------------------------------
# Geometry
# --------------------------------------------------------------------------

def support_surface(grid, exclude, nodes=None):
    """Highest supporting terrain top per cell (0 = generic ground plane)."""
    out = np.zeros((grid.ny, grid.nx))
    for n in (nodes or SUPPORT_NODES):
        if n in exclude:
            continue
        tris, _ = native_triangles(node_name(n))
        top = core.raster_surface(grid, tris, upward=True)
        out = np.where(~np.isnan(top) & (top > out), top, out)
    return out


def build_heightfield_mesh(grid, inside, top, base, edge=1.0, radius=0.0, neighbour=None, vsmooth=True):
    """Closed solid between ``base`` and ``top`` over the inside cells.

    Cells become lattice quads; the staircase outline is relaxed in 2D so
    rock outlines follow the carved footprint without pixel steps.
    """
    inside = core.remove_pinches(inside)
    ny, nx = inside.shape
    index = -np.ones((ny + 1, nx + 1), int)
    corners = []
    for j, i in zip(*np.nonzero(inside)):
        for cj, ci in ((j, i), (j, i + 1), (j + 1, i + 1), (j + 1, i)):
            if index[cj, ci] < 0:
                index[cj, ci] = len(corners)
                corners.append((ci, cj))
    corners = np.array(corners)
    xy = np.stack([grid.x0 + corners[:, 0] * grid.cell, grid.y0 + corners[:, 1] * grid.cell], 1)
    # Corner heights: mean of the adjacent inside cells.
    zt = np.zeros(len(corners))
    zb = np.zeros(len(corners))
    cnt = np.zeros(len(corners))
    for j, i in zip(*np.nonzero(inside)):
        for cj, ci in ((j, i), (j, i + 1), (j + 1, i + 1), (j + 1, i)):
            k = index[cj, ci]
            zt[k] += top[j, i]
            zb[k] += base[j, i]
            cnt[k] += 1
    zt /= cnt
    zb /= cnt
    quads = [(index[j, i], index[j, i + 1], index[j + 1, i + 1], index[j + 1, i])
             for j, i in zip(*np.nonzero(inside))]
    count = {}
    for q in quads:
        for e in ((q[0], q[1]), (q[1], q[2]), (q[2], q[3]), (q[3], q[0])):
            key = tuple(sorted(e))
            count[key] = count.get(key, 0) + 1
    boundary = {k for k, v in count.items() if v == 1}
    neighbours = {}
    for a, b in boundary:
        neighbours.setdefault(a, []).append(b)
        neighbours.setdefault(b, []).append(a)
    ring = np.array(sorted(neighbours))
    if any(len(v) != 2 for v in neighbours.values()):
        raise ValueError('Outline is not a set of simple loops')
    # Relax the staircase: boundary corners move toward their loop neighbours.
    for _ in range(6):
        target = np.array([xy[neighbours[k]].mean(0) for k in ring])
        xy[ring] = 0.5 * xy[ring] + 0.5 * target
    # Relax interior corners toward their lattice neighbours so the first
    # interior ring follows the smoothed outline.
    adjacency = {}
    for q in quads:
        for u, v in ((q[0], q[1]), (q[1], q[2]), (q[2], q[3]), (q[3], q[0])):
            adjacency.setdefault(u, set()).add(v)
            adjacency.setdefault(v, set()).add(u)
    interior = np.array([k for k in range(len(xy)) if k not in neighbours])
    if len(interior):
        nbrs = [list(adjacency[k]) for k in interior]
        for _ in range(3):
            target = np.array([xy[nb].mean(0) for nb in nbrs])
            xy[interior] = 0.5 * xy[interior] + 0.5 * target
    # Rounded shoulders from the continuous distance to the relaxed outline:
    # the outline keeps ``edge`` of the local height (a short skirt) and the
    # full height is reached ``radius`` native units inside (sqrt profile).
    if radius > 0:
        # Only outline segments facing the source camera (south) or sideways
        # are rounded. North-facing (back) crests define the top of the painted
        # silhouette and keep their native height.
        edge_quad = {}
        for q in quads:
            for u, v in ((q[0], q[1]), (q[1], q[2]), (q[2], q[3]), (q[3], q[0])):
                key = tuple(sorted((u, v)))
                if key in boundary:
                    edge_quad[key] = q
        keep_segments = []
        for key in boundary:
            a, b = xy[key[0]], xy[key[1]]
            centre = xy[list(edge_quad[key])].mean(0)
            d = b - a
            normal = np.array([d[1], -d[0]])
            if np.dot(normal, (a + b) / 2 - centre) < 0:
                normal = -normal
            normal /= max(np.linalg.norm(normal), 1e-9)
            if neighbour is not None:
                probe = (a + b) / 2 + normal * grid.cell
                i = int((probe[0] - grid.x0) / grid.cell)
                j = int((probe[1] - grid.y0) / grid.cell)
                if 0 <= i < grid.nx and 0 <= j < grid.ny and neighbour[j, i]:
                    continue  # seam shared with an adjoining rock of this asset
            if normal[1] > -0.35:
                keep_segments.append(key)
        if not keep_segments:
            keep_segments = list(boundary)
        seg_a = xy[[a for a, b in keep_segments]]
        seg_b = xy[[b for a, b in keep_segments]]
        dist = np.full(len(xy), np.inf)
        for start in range(0, len(xy), 512):
            p = xy[start:start + 512, None, :]
            ab = seg_b - seg_a
            t = np.clip(((p - seg_a) * ab).sum(-1) / np.maximum((ab * ab).sum(-1), 1e-12), 0, 1)
            closest = seg_a + t[..., None] * ab
            dist[start:start + 512] = np.sqrt(((p - closest) ** 2).sum(-1)).min(1)
        profile = edge + (1 - edge) * np.sin(0.5 * np.pi * np.clip(dist / radius, 0, 1))
        zt = zb + (zt - zb) * profile
    # Vertex-level Laplacian smoothing of the cap (outline fixed) removes
    # lattice zigzag where the diagonal outline meets the grid.
    if len(interior) and vsmooth:
        for _ in range(4):
            target = np.array([zt[nb].mean() for nb in nbrs])
            zt[interior] = 0.5 * zt[interior] + 0.5 * target
    zt = np.maximum(zt, zb + 0.3)
    n = len(xy)
    native = np.vstack([np.column_stack([xy, zt]), np.column_stack([xy, zb])])
    polys = []
    for q in quads:
        polys.append(q)
        polys.append(tuple(v + n for v in reversed(q)))
        for u, v in ((q[0], q[1]), (q[1], q[2]), (q[2], q[3]), (q[3], q[0])):
            if tuple(sorted((u, v))) in boundary:
                polys.append((v, u, u + n, v + n))
    return native, polys


def restore_native(obj):
    """Make the mesh equal to the frozen native geometry; True if it changed."""
    record = _INVENTORY[obj['source_node']]['geometry']
    same = (len(record['vertices']) == len(obj.data.vertices)
            and all((Vector(a) - v.co).length < 1e-3 for a, v in zip(record['vertices'], obj.data.vertices))
            and [list(p.vertices) for p in obj.data.polygons] == record['faces'])
    if same:
        return False
    old = obj.data
    mesh = bpy.data.meshes.new(obj.name)
    mesh.from_pydata([tuple(v) for v in record['vertices']], [], [tuple(f) for f in record['faces']])
    mesh.update()
    mesh.materials.append(old.materials[0])
    uv = mesh.uv_layers.new(name='UVMap')
    for loop in mesh.loops:
        p = obj.matrix_world @ mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv = (p.x / 2944, 1 - (-p.y * core.SIN35 - p.z * core.COS35) / 2176)
    obj.data = mesh
    if old.users == 0:
        bpy.data.meshes.remove(old)
    mesh.name = obj.name
    return True


def neighbour_cells(grid, nodes):
    out = np.zeros((grid.ny, grid.nx), bool)
    for node in nodes:
        tris, _ = native_triangles(node)
        out |= ~np.isnan(core.raster_surface(grid, tris))
    return core.close_cells(out, 1) if out.any() else out


def stump_mesh(tris, spec):
    """Round tree stump: a flared frustum inscribed in the native footprint."""
    xy = tris.reshape(-1, 3)
    cx, cy = (xy[:, 0].min() + xy[:, 0].max()) / 2, (xy[:, 1].min() + xy[:, 1].max()) / 2
    # Round in metric world space: native y is foreshortened by sin 35.
    radius = min(xy[:, 0].max() - xy[:, 0].min(), (xy[:, 1].max() - xy[:, 1].min()) / core.SIN35) / 2
    top = float(xy[:, 2].max())
    rings = [(0.0, 1.0), (0.12 * top, 0.9), (0.85 * top, 0.8), (top, 0.74)]
    count = 16
    verts = []
    for z, f in rings:
        for k in range(count):
            a = 2 * math.pi * k / count
            verts.append((cx + radius * f * math.cos(a), cy + radius * f * core.SIN35 * math.sin(a), z))
    verts.append((cx, cy, top))
    verts.append((cx, cy, 0.0))
    world = core.native_to_world(np.array(verts))
    bm = bmesh.new()
    bv = [bm.verts.new(Vector(p)) for p in world]
    for r in range(len(rings) - 1):
        for k in range(count):
            a, b = r * count + k, r * count + (k + 1) % count
            bm.faces.new([bv[a], bv[b], bv[b + count], bv[a + count]])
    cap, bottom = bv[-2], bv[-1]
    last = (len(rings) - 1) * count
    for k in range(count):
        bm.faces.new([bv[last + k], bv[last + (k + 1) % count], cap])
        bm.faces.new([bv[(k + 1) % count], bv[k], bottom])
    return bm, {'centre': [round(cx, 2), round(cy, 2)], 'radius': round(radius, 2), 'top': round(top, 2),
                'rings_height_radiusfraction': rings}


def tile_mesh(bm, size):
    """Cut large dissolved faces on a world-space XY grid (bounded projection islands)."""
    xs = [v.co.x for v in bm.verts]
    ys = [v.co.y for v in bm.verts]
    for axis, lo, hi in ((0, min(xs), max(xs)), (1, min(ys), max(ys))):
        k = math.floor(lo / size) + 1
        while k * size < hi:
            co = [0.0, 0.0, 0.0]
            co[axis] = k * size
            no = [0.0, 0.0, 0.0]
            no[axis] = 1.0
            bmesh.ops.bisect_plane(bm, geom=bm.verts[:] + bm.edges[:] + bm.faces[:], dist=1e-4,
                                   plane_co=co, plane_no=no)
            k += 1


def finalize(bm, label):
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-4)
    bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=1e-5)
    bmesh.ops.triangulate(bm, faces=bm.faces)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    defects = {'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
               'boundary_edges': sum(e.is_boundary for e in bm.edges),
               'degenerate_faces': sum(f.calc_area() < 1e-6 for f in bm.faces)}
    return defects


WALL_LANES = ('south_gate_walls', 'east_gate_walls')
WEST_LANES = ('west_complex',)
BASTION_WALLS = [282, 283, 284]


# Terrain fronts. Plateau tops stay at z = 220 wherever the art allows. A
# column is lowered only while its top pixel lands on the reviewed silhouette
# of a neighbouring wall (south/east/west lanes): the artwork shows that
# masonry, so the terrain must recede below the wall foot there.
def terrain_front(lanes=None, **kw):
    spec = {'mode': 'front', 'cell': 4.0, 'support': False, 'floor': 0.0, 'round': 0.0,
            'edge': 1.0, 'smooth': 0, 'min_component': 6, 'fill': False, 'dissolve': True,
            'vsmooth': False, 'wall_lanes': lanes or WALL_LANES}
    spec.update(kw)
    return spec


NODES[455] = terrain_front(cell=2.0, foreign_masks=[138, 139])
NODES[456] = terrain_front(cell=2.0, foreign_masks=[138, 139])
# Round 2 (integrated neighbours): terrain and rock columns additionally recede
# wherever they would stand in front of a neighbour's reviewed receiver that
# the integrated scene now shows behind them ('unhide').
NODES[62] = terrain_front(unhide=True, dissolve_tolerance=2)
NODES[63] = terrain_front(unhide=True)
NODES[64] = terrain_front(unhide=True)
NODES[70] = terrain_front(cell=2.0, unhide=True)
NODES[53] = terrain_front(no_front=True, unhide=True)
NODES[52] = terrain_front(cell=2.0, no_front=True, unhide=True, unhide_despike=True)
NODES[57] = terrain_front(cell=2.0, no_front=True, unhide=True, unhide_edge_trim=True)
# West plateau: the bastion's lower rooms (terrace volumes 060/069, floor
# z = 147) are cut out of it, and its front below the western complex is
# carved to the moat-bank envelope with the bastion walls treated as foreign.
NODES[65] = terrain_front(WEST_LANES + WALL_LANES, cutout=[60, 69], unhide=True)
# South-western cliff rocks meet the bastion wall feet (traced z 95-150 by the
# western complex lane): carve their band in front of the bastion likewise.
for _n in (423, 424, 425, 426, 427, 428, 429, 430, 431):
    # Foliage exclusions hide rock (support it); bastion wall exclusions do
    # not, so the cliff recedes below the bastion's painted wall feet.
    NODES[_n] = rock(cell=2.0, support=True, round=0.0, edge=1.0, smooth=1, fill=True, dissolve=True,
                     vsmooth=True, occluders=CLIFF_FOLIAGE, occluders_only=True, unhide=True)


def wall_nodes(lanes):
    assignments = json.loads((ROOT / 'worker-assignments.json').read_text())['assignments']
    catalog = json.loads((ROOT / 'grouping/catalog.json').read_text())
    groups = {g['id']: g for g in catalog['groups']}
    return sorted({node_name(p['obstacle']) for lane in lanes for asset in assignments[lane]
                   for p in groups[asset]['parts']})


def front_band(grid, lanes, depth):
    """Cells south of the southernmost wall cell of each column (within depth)."""
    walls = np.zeros((grid.ny, grid.nx), bool)
    for node in wall_nodes(lanes):
        tris, _ = native_triangles(node)
        xy = tris.reshape(-1, 3)
        if xy[:, 0].max() < grid.x0 or xy[:, 0].min() > grid.x0 + grid.nx * grid.cell:
            continue
        walls |= ~np.isnan(core.raster_surface(grid, tris))
    band = np.zeros_like(walls)
    rows = np.arange(grid.ny)[:, None]
    has = walls.any(0)
    last = np.where(has, grid.ny - 1 - np.argmax(walls[::-1], 0), -1)
    band = (rows > last[None, :]) & has[None, :] & ((rows - last[None, :]) * grid.cell <= depth)
    return band


_WALL_CACHE = {}


def wall_silhouettes(masks, lanes):
    """Union of the frozen reviewed include masks of every wall-lane node."""
    key = tuple(lanes)
    if key not in _WALL_CACHE:
        nodes = set(wall_nodes(lanes))
        frozen = json.loads((ROOT / 'mask-review/source-masks-v1.json').read_text())
        indices = sorted({i for r in frozen['projections']['exterior']['assignments']
                          if r.get('source_node') in nodes for i in r['mask_indices'] if i != 428})
        _WALL_CACHE[key] = masks.union(indices)
    return _WALL_CACHE[key]


def raise_to_trace(grid, heights, inside, base, trace):
    """Raise a rock crest to a traced painted crest line.

    ``trace['points']`` are [pixel x, pixel y, native y of the contact]; the
    crest height at each point is contact_y - pixel_y (source camera). South
    of the contact line the surface falls off with ``slope`` (z per native y)
    for at most ``reach`` units and never drops below the existing height.
    """
    pts = np.array(trace['points'], float)
    X, Y = grid.centers()
    in_range = (X >= pts[:, 0].min()) & (X <= pts[:, 0].max())
    depth = np.interp(X, pts[:, 0], pts[:, 2])
    crest = np.interp(X, pts[:, 0], pts[:, 2] - pts[:, 1])
    target = crest - trace['slope'] * np.maximum(Y - depth, 0)
    zone = in_range & (Y >= depth) & (Y <= depth + trace['reach']) & (target > base + 0.5)
    new = np.where(zone, np.fmax(heights, target), heights)
    raised = int((zone & ~(inside & (np.nan_to_num(heights) >= target))).sum())
    return new, inside | zone, raised, np.where(zone, new, np.nan)


def rebuild(obj, spec, masks, working_assignment, dry=False, neighbours=(), context=None):
    n = int(obj['source_node'].split('-')[-1])
    tris, matrix = native_triangles(obj['source_node'])
    if max(abs(a - b) for ra, rb in zip(matrix, obj.matrix_world) for a, b in zip(ra, rb)) > 1e-4:
        raise ValueError(f'Native transform differs for {obj.name}')
    report = {'object': obj.name, 'source_node': obj['source_node'], 'mode': spec['mode']}
    ztop_native = float(tris[..., 2].max())
    report['native_top'] = round(ztop_native, 2)
    if spec['mode'] == 'keep':
        report['changed'] = restore_native(obj)
        return report, None
    if spec['mode'] == 'bury':
        nat = tris.reshape(-1, 3)
        if dry:
            return report, None
        lo = spec['top'] - spec['depth']
        pts = np.unique(np.round(nat[:, :2], 3), axis=0)
        centre = pts.mean(0)
        order = np.argsort(np.arctan2(pts[:, 1] - centre[1], pts[:, 0] - centre[0]))
        ring = [p for p in pts[order]]
        hull = []
        for p in ring:  # drop collinear/duplicate corners
            if not hull or np.linalg.norm(p - hull[-1]) > 0.05:
                hull.append(p)
        n = len(hull)
        world = core.native_to_world(np.array([[p[0], p[1], lo] for p in hull] + [[p[0], p[1], spec['top']] for p in hull]))
        bm = bmesh.new()
        verts = [bm.verts.new(Vector(p)) for p in world]
        bm.faces.new(verts[:n][::-1])
        bm.faces.new(verts[n:])
        for i in range(n):
            j = (i + 1) % n
            bm.faces.new([verts[i], verts[j], verts[n + j], verts[n + i]])
        report['construction'] = (f"native footprint kept, sunk to z {lo}-{spec['top']} inside the plateau "
                                  '(collision-only marker, never visible)')
        report['topology'] = finalize(bm, obj.name)
        inverse = obj.matrix_world.inverted()
        for v in bm.verts:
            v.co = inverse @ v.co
        return report, bm
    if spec['mode'] == 'stump':
        if dry:
            return report, None
        bm, shape = stump_mesh(tris, spec)
        report['construction'] = 'round flared stump inscribed in the native footprint'
        report['stump'] = shape
        report['topology'] = finalize(bm, obj.name)
        inverse = obj.matrix_world.inverted()
        for v in bm.verts:
            v.co = inverse @ v.co
        return report, bm
    cell = spec['cell']
    xy = tris.reshape(-1, 3)
    grid = core.Grid(xy[:, 0].min(), xy[:, 1].min(), xy[:, 0].max(), xy[:, 1].max(), cell)
    top = core.raster_surface(grid, tris, upward=True)
    # Close one-cell seams between the native faces of multi-part prisms.
    native_inside = core.close_cells(~np.isnan(top), 2)
    top = core.extend(top, ~np.isnan(top), 2)
    if spec.get('cutout'):
        cut = np.zeros_like(native_inside)
        for m in spec['cutout']:
            cut |= ~np.isnan(core.raster_surface(grid, native_triangles(node_name(m))[0]))
        report['cutout_cells'] = int((native_inside & cut).sum())
        report['cutout_nodes'] = spec['cutout']
        native_inside &= ~cut
    top = np.where(native_inside, top, np.nan)
    support = support_surface(grid, exclude={n}, nodes=spec.get('support_nodes')) if spec['support'] else np.zeros_like(top)
    base = np.maximum(support, spec['floor'])
    buried = native_inside & (top <= base + 0.5)
    report['native_cells'] = int(native_inside.sum())
    report['buried_cells'] = int(buried.sum())
    report['support_used'] = spec['support']
    report['floor'] = spec['floor']
    report['base_range'] = [round(float(base[native_inside & ~buried].min()), 1),
                            round(float(base[native_inside & ~buried].max()), 1)] if (native_inside & ~buried).any() else None
    if spec['mode'] == 'front' and spec.get('no_front'):
        heights = np.where(native_inside, top, np.nan)
        report['front_carve'] = {'native_columns': int(native_inside.sum()), 'lowered_columns': 0,
                                 'dropped_to_base': 0}
        inside = native_inside.copy()
    elif spec['mode'] == 'front':
        if spec.get('foreign_masks'):
            foreign = masks.union(spec['foreign_masks'])
            eligible = native_inside
            report['foreign_masks'] = spec['foreign_masks']
        else:
            foreign = wall_silhouettes(masks, spec['wall_lanes'])
            eligible = native_inside & front_band(grid, spec['wall_lanes'], spec.get('band_depth', 120.0))
        report['band_cells'] = int(eligible.sum())
        carved, counts = core.carve_foreign(grid, np.where(eligible, top, np.nan), base, foreign)
        heights = np.where(eligible, carved, np.where(native_inside, top, np.nan))
        report['front_carve'] = counts
        report['wall_lanes'] = [] if spec.get('foreign_masks') else list(spec['wall_lanes'])
        inside = native_inside & ~np.isnan(heights) & (heights > base + 0.5)
    elif spec['mode'] == 'trim':
        values = sorted(set(np.round(base[native_inside & ~buried], 1).tolist()))
        report['trim_base_values'] = values[:8]
        heights = top
        inside = native_inside & ~buried
    else:
        own = masks.union([i for i in working_assignment['mask_indices'] if i != 428])
        reviewed_exclusions = working_assignment.get('exclude_mask_indices', [])
        if reviewed_exclusions:
            own &= ~masks.union(reviewed_exclusions)
        # Terrain fronts treat neighbouring walls as foreign (the wall masonry
        # must not be hidden behind terrain); rocks treat exclusions as hidden.
        excluded = list(spec['occluders']) if spec.get('occluders_only') else reviewed_exclusions + spec['occluders']
        occ = masks.union(excluded) if excluded else np.zeros_like(own)
        eligible = native_inside
        if spec.get('band'):
            eligible = native_inside & front_band(grid, spec['band'], spec.get('band_depth', 1e9))
            report['band_cells'] = int(eligible.sum())
        carved, counts = core.carve(grid, np.where(eligible, top, np.nan), base, own, occ)
        heights = np.where(eligible, carved, np.where(native_inside, top, np.nan))
        report['carve'] = counts
        report['own_masks'] = working_assignment['mask_indices']
        report['occluder_masks'] = excluded
        inside = ~np.isnan(heights) & (heights > base + 0.5)
    raise_cap = None
    if spec.get('raise'):
        heights, inside, raised, raise_cap = raise_to_trace(grid, heights, inside, base, spec['raise'])
        report['raised_cells'] = raised
        report['raise_trace'] = spec['raise']
    labels, sizes = core.components(inside)
    keep = np.zeros_like(inside)
    for label, size in enumerate(sizes):
        if label and size >= spec['min_component']:
            keep |= labels == label
    inside = core.remove_pinches(core.fill_holes(keep) if spec.get('fill', True) else keep)
    filled = inside & (np.isnan(heights) | (heights <= base + 0.5))
    heights = core.extend(np.where(inside & ~filled, heights, np.nan), inside & ~filled, 6)
    heights = np.where(inside, heights, np.nan)
    if np.isnan(heights[inside]).any():
        raise ValueError(f'{obj.name}: unresolved heights after hole fill')
    if spec.get('dome'):
        heights = np.where(inside, core.mirror_dome(np.nan_to_num(heights), inside), np.nan)
        heights = np.maximum(heights, base + 0.5)
    if spec['smooth']:
        heights = np.where(inside, core.smooth(np.nan_to_num(heights), inside, spec['smooth']), np.nan)
    if spec['mode'] not in ('trim', 'front'):
        cap = np.where(native_inside, top, heights)
        if raise_cap is not None:
            cap = np.where(~np.isnan(raise_cap), np.fmax(cap, raise_cap), cap)
        heights = np.minimum(heights, cap)
    if spec.get('unhide'):
        if context is None:
            raise ValueError(f'{obj.name}: unhide requires the neighbour depth context')
        receiver = context.receiver(set(working_assignment['mask_indices']), masks, spec.get('unhide_margin', 0))
        prior = np.where(inside, heights, np.nan)
        heights, lowered = core.unhide(grid, prior, inside, base, receiver,
                                       context.depth, context.x0, context.y0)
        if spec.get('unhide_edge_trim'):
            # Replace the per-column cut by a clean vertical retreat of the
            # north edge: per x column, drop every cell north of the southern-
            # most lowered cell (window-max over 5 columns), keep the rest at
            # its full height.
            cut = inside & (heights < prior - 0.5)
            ys = np.arange(grid.ny)[:, None] * np.ones((1, grid.nx), int)
            last = np.where(cut.any(0), np.where(cut, ys, -1).max(0), -1)
            smooth_last = last.copy()
            for k in range(grid.nx):
                smooth_last[k] = last[max(0, k - 2):k + 3].max()
            drop = ys <= smooth_last[None, :]
            report['unhide_edge_trim_cells'] = int((inside & drop).sum())
            inside = inside & ~drop
            heights = np.where(inside, prior, np.nan)
        if spec.get('unhide_despike'):
            # Lower-only 3x3 minimum around lowered cells removes the one-cell
            # sawtooth the per-column cut leaves along long edges.
            cut = inside & (heights < prior - 0.5)
            ring = cut.copy()
            ring[1:, :] |= cut[:-1, :]
            ring[:-1, :] |= cut[1:, :]
            ring[:, 1:] |= cut[:, :-1]
            ring[:, :-1] |= cut[:, 1:]
            filled = np.where(inside, heights, np.inf)
            low = filled.copy()
            for dy in (-1, 0, 1):
                for dx in (-1, 0, 1):
                    low = np.minimum(low, np.roll(np.roll(filled, dy, 0), dx, 1))
            low = np.maximum(low, base)
            heights = np.where(ring & inside & np.isfinite(low), np.minimum(heights, low), heights)
        inside = inside & (heights > base + 0.5)
        labels, sizes = core.components(inside)
        inside = core.break_pinches(np.isin(labels, [k for k, v in enumerate(sizes) if k and v >= 4]))
        heights = np.where(inside, heights, np.nan)
        report['unhide_lowered_columns'] = lowered
    report['kept_cells'] = int(inside.sum())
    report['kept_height_range'] = [round(float(np.nanmin(heights)), 1), round(float(np.nanmax(heights)), 1)] if inside.any() else None
    if not inside.any():
        raise ValueError(f'{obj.name}: rebuild removed every column')
    if dry:
        return report, None
    raised = native_inside & (base > 0.5)
    if (spec['mode'] == 'front' and report.get('front_carve', {}).get('lowered_columns', 1) == 0
            and not report.get('unhide_lowered_columns')):
        report['construction'] = 'native volume kept: no column covers neighbouring wall masonry'
        report['changed'] = restore_native(obj)
        return report, None
    if spec['mode'] == 'trim' and raised.sum() <= 0.05 * native_inside.sum():
        report['raised_base_fraction'] = round(float(raised.sum() / native_inside.sum()), 4)
        report['construction'] = ('native volume kept: its footprint stands on the z = 0 ground '
                                  '(any overlap with supporting terrain is under 5 percent)')
        report['changed'] = restore_native(obj)
        return report, None
    native, polys = build_heightfield_mesh(grid, inside, heights, base, spec.get('edge', 1.0),
                                           spec['round'], neighbour_cells(grid, neighbours),
                                           spec.get('vsmooth', True))
    world = core.native_to_world(native)
    bm = bmesh.new()
    verts = [bm.verts.new(Vector(p)) for p in world]
    for p in polys:
        try:
            bm.faces.new([verts[i] for i in p])
        except ValueError:
            pass
    report['construction'] = ('closed native-top solid over the unburied footprint, based on the supporting terrain'
                              if spec['mode'] == 'trim' else 'closed heightfield solid over carved native footprint')
    defects = None
    if spec.get('dissolve'):
        # Merge coplanar lattice faces; keep the undissolved lattice if the
        # result is not a clean closed shell.
        trial = bm.copy()
        bmesh.ops.remove_doubles(trial, verts=trial.verts, dist=1e-4)
        bmesh.ops.dissolve_limit(trial, angle_limit=math.radians(0.5), verts=trial.verts, edges=trial.edges)
        tile_mesh(trial, spec.get('tile', 48.0))
        trial_defects = finalize(trial, obj.name)
        # A single triangulation artefact on a very large dissolved terrain is
        # accepted (and reported) when the lattice alone would overflow the
        # frozen ownership atlas.
        tolerable = (spec.get('dissolve_tolerance', 0) >= trial_defects['nonmanifold_edges']
                     and not trial_defects['boundary_edges'] and not trial_defects['degenerate_faces'])
        if tolerable and trial_defects['nonmanifold_edges']:
            report['topology_exception'] = (f"{trial_defects['nonmanifold_edges']} non-manifold edge(s) from "
                                            'triangulating the dissolved flat top; accepted to keep the '
                                            'ownership atlas within 16384 px')
        if any(trial_defects.values()) and not tolerable:
            trial.free()
            report['dissolve_skipped'] = trial_defects
        else:
            bm.free()
            bm, defects = trial, trial_defects
    if defects is None:
        defects = finalize(bm, obj.name)
    report['topology'] = defects
    inverse = obj.matrix_world.inverted()
    for v in bm.verts:
        v.co = inverse @ v.co
    return report, bm


class NeighbourContext:
    """Source-camera z-buffer of every visible non-owned mesh around the asset.

    ``receiver(own_masks)`` marks pixels whose front-most neighbour owns a
    reviewed mask covering that pixel (masks the node includes itself are
    ignored: shared composites are resolved by first-hit gating instead).
    """

    def __init__(self, collection, owned, masks, rows, part_ids, margin=40):
        pts = []
        for obj in owned:
            tris, _ = native_triangles(obj['source_node'])
            nat = tris.reshape(-1, 3)
            pts.append(np.stack([nat[:, 0], nat[:, 1] - nat[:, 2]], 1))
            pts.append(np.stack([nat[:, 0], nat[:, 1]], 1))
        pts = np.vstack(pts)
        self.x0 = max(0, int(pts[:, 0].min()) - margin)
        self.y0 = max(0, int(pts[:, 1].min()) - margin)
        x1 = min(2944, int(pts[:, 0].max()) + margin)
        y1 = min(2176, int(pts[:, 1].max()) + margin)
        shape = (y1 - self.y0, x1 - self.x0)
        self.depth = np.full(shape, -np.inf)
        label_of = np.zeros(shape, int)
        names = ['']
        owned_set = set(owned)
        for obj in collection.all_objects:
            if obj.type != 'MESH' or obj.hide_render or obj in owned_set:
                continue
            tris = np.array(object_world_triangles(obj), float).reshape(-1, 3, 3)
            if not len(tris):
                continue
            sx = tris[..., 0]
            sy = -tris[..., 1] * core.SIN35 - tris[..., 2] * core.COS35
            if sx.max() < self.x0 or sx.min() > x1 or sy.max() < self.y0 or sy.min() > y1:
                continue
            names.append(obj.get('source_node') or '')
            core.zbuffer(self.depth, label_of, tris, len(names) - 1, self.x0, self.y0)
        front_node = np.array(names, object)[label_of]
        self.selections = {}
        owners = {}
        for node, row in rows.items():
            if node in part_ids:
                continue
            for i in row['mask_indices']:
                if i != 428:
                    owners.setdefault(i, set()).add(node)
        for i, nodes in owners.items():
            r = masks.records[i]
            bx, by = r['box_top_left']
            bw, bh = r['box_size']
            if bx > x1 or bx + bw < self.x0 or by > y1 or by + bh < self.y0:
                continue
            sel = masks.get(i)[self.y0:y1, self.x0:x1] & np.isin(front_node, list(nodes))
            if sel.any():
                self.selections[i] = sel
        self.shape = shape

    def receiver(self, own_masks, masks, margin=0):
        """Neighbour receiver pixels (optionally dilated), minus the node's own silhouette."""
        out = np.zeros(self.shape, bool)
        for i, sel in self.selections.items():
            if i not in own_masks:
                out |= sel
        for _ in range(margin):
            grown = out.copy()
            grown[1:, :] |= out[:-1, :]
            grown[:-1, :] |= out[1:, :]
            grown[:, 1:] |= out[:, :-1]
            grown[:, :-1] |= out[:, 1:]
            out = grown
        own = [i for i in own_masks if i != 428]
        if own:
            y1, x1 = self.y0 + self.shape[0], self.x0 + self.shape[1]
            out &= ~masks.union(own)[self.y0:y1, self.x0:x1]
        return out


def _island_height(points):
    """Height of the per-face projection island the frozen bake packs into shelves."""
    o = points[0]
    normal = (points[1] - o).cross(points[2] - o)
    if normal.length < 1e-12:
        return 0.0
    normal.normalize()
    axis = max((p - o for p in points), key=lambda v: v.length).normalized()
    vertical = normal.cross(axis).normalized()
    ys = [(p - o).dot(vertical) for p in points]
    return max(ys) - min(ys)


# Painted arch opening of the south-western ravine bridge, traced on the
# unmarked covered art (pixel x, y), on the east face of arch wall 075.
BRIDGE_ARCH_PIXELS = [(673, 1960), (673, 1920), (675, 1912), (678, 1906), (683, 1901), (690, 1898),
                      (697, 1899), (702, 1903), (706, 1910), (708, 1918), (708, 1960)]
BRIDGE_ARCH_FACE = ((659.8, 2057.3), (716.4, 1986.0))  # native footprint edge of 075's east face
BRIDGE_ARCH_NODES = ('building-057', 'building-075')


def arch_cutter(depth=180.0, outset=6.0):
    """Closed prism: the traced arch outline on the face, pushed through the bridge."""
    (ax, ay), (bx, by) = BRIDGE_ARCH_FACE
    ux, uy = bx - ax, by - ay
    length = math.hypot(ux, uy)
    nx, ny = uy / length, -ux / length  # outward (south-east) normal in native xy
    if nx < 0:
        nx, ny = -nx, -ny
    ring = []
    for px, py in BRIDGE_ARCH_PIXELS:
        t = (px - ax) / ux
        y = ay + t * uy
        ring.append((px, y, y - py))
    front = [(x + nx * outset, y + ny * outset, z) for x, y, z in ring]
    back = [(x - nx * depth, y - ny * depth, z) for x, y, z in ring]
    nat = np.array(front + back)
    world = core.native_to_world(nat)
    n = len(ring)
    mesh = bpy.data.meshes.new(TAG + ' arch cutter')
    faces = [tuple(range(n)), tuple(range(2 * n - 1, n - 1, -1))]
    faces += [(i, (i + 1) % n, n + (i + 1) % n, n + i) for i in range(n)]
    mesh.from_pydata([tuple(p) for p in world], [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    return mesh, {'traced_pixels': BRIDGE_ARCH_PIXELS, 'face_edge_native_xy': BRIDGE_ARCH_FACE,
                  'native_outline_xyz': [[round(c, 1) for c in p] for p in ring],
                  'passage_depth': depth}


def cut_bridge_arch(objects):
    """Boolean the painted arch passage out of the bridge deck 057 and arch wall 075."""
    mesh, record = arch_cutter()
    cutter = bpy.data.objects.new(TAG + ' arch cutter', mesh)
    bpy.context.scene.collection.objects.link(cutter)
    results = {}
    for obj in objects:
        before = sum(p.area for p in obj.data.polygons)
        # Native prisms carry 0.1-unit duplicate corners and open caps; weld
        # and close them so the boolean yields a closed shell.
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=0.3)
        open_edges = [e for e in bm.edges if e.is_boundary]
        if open_edges:
            bmesh.ops.holes_fill(bm, edges=open_edges, sides=0)
        bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=1e-4)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        bm.to_mesh(obj.data)
        bm.free()
        obj.data.update()
        mod = obj.modifiers.new('arch passage', 'BOOLEAN')
        mod.operation = 'DIFFERENCE'
        mod.solver = 'EXACT'
        mod.object = cutter
        depsgraph = bpy.context.evaluated_depsgraph_get()
        new = bpy.data.meshes.new_from_object(obj.evaluated_get(depsgraph))
        obj.modifiers.remove(mod)
        old = obj.data
        new.materials.clear()
        for m in old.materials:
            new.materials.append(m)
        obj.data = new
        if old.users == 0:
            bpy.data.meshes.remove(old)
        new.name = obj.name
        bm = bmesh.new()
        bm.from_mesh(new)
        record_obj = {'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
                      'boundary_edges': sum(e.is_boundary for e in bm.edges),
                      'faces': len(bm.faces), 'area_before': round(before, 1),
                      'area_after': round(sum(f.calc_area() for f in bm.faces), 1)}
        bm.free()
        results[obj['source_node']] = record_obj
    bpy.data.objects.remove(cutter)
    bpy.data.meshes.remove(mesh)
    record['results'] = results
    return record


def apply_mesh(obj, bm):
    old = obj.data
    mesh = bpy.data.meshes.new(obj.name)
    # Order faces by projection-island height (tallest first) so the frozen
    # shelf packer fills rows evenly; geometry is unchanged.
    world = obj.matrix_world
    verts = [v.co.copy() for v in bm.verts]
    bm.verts.index_update()
    faces = [[v.index for v in f.verts] for f in bm.faces]
    bm.free()
    heights = [_island_height([world @ verts[i] for i in f]) for f in faces]
    order = sorted(range(len(faces)), key=lambda k: -heights[k])
    mesh.from_pydata(verts, [], [faces[k] for k in order])
    mesh.update()
    # Keep the original fallback material only; the frozen projection helper
    # rebuilds its own projection materials and UV layers.
    mesh.materials.append(old.materials[0])
    uv = mesh.uv_layers.new(name='UVMap')
    for loop in mesh.loops:
        p = obj.matrix_world @ mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv = (p.x / 2944, 1 - (-p.y * core.SIN35 - p.z * core.COS35) / 2176)
    obj.data = mesh
    if old.users == 0:
        bpy.data.meshes.remove(old)
    mesh.name = obj.name


def pixel_triangles(world_tris):
    """Source-pixel triangles (x, y - z) from world-space triangles."""
    w = np.asarray(world_tris, float).reshape(-1, 3)
    px = np.stack([w[:, 0], -w[:, 1] * core.SIN35 - w[:, 2] * core.COS35, np.zeros(len(w))], 1)
    return px.reshape(-1, 3, 3)


def object_world_triangles(obj):
    mesh = obj.data
    mesh.calc_loop_triangles()
    mw = obj.matrix_world
    co = [mw @ v.co for v in mesh.vertices]
    return [[list(co[i]) for i in t.vertices] for t in mesh.loop_triangles]


def silhouette_stats(owned, assignments, masks):
    """Asset silhouette (native vs rebuilt) against its reviewed native masks."""
    native_tris, new_tris, own = [], [], None
    for obj in owned:
        tris, _ = native_triangles(obj['source_node'])
        native_tris.extend(core.native_to_world(tris.reshape(-1, 3)).reshape(-1, 3, 3).tolist())
        new_tris.extend(object_world_triangles(obj))
        row = assignments[obj['source_node']]
        idx = [i for i in row['mask_indices'] if i != 428]
        if not idx:
            continue
        m = masks.union(idx)
        if row.get('exclude_mask_indices'):
            m &= ~masks.union(row['exclude_mask_indices'])
        own = m if own is None else own | m
    if own is None:
        return {'status': 'no reviewed native mask; silhouette comparison not applicable'}
    a = pixel_triangles(native_tris)
    b = pixel_triangles(new_tris)
    both = np.vstack([a.reshape(-1, 3), b.reshape(-1, 3)])
    grid = core.Grid(both[:, 0].min(), both[:, 1].min(), both[:, 0].max(), both[:, 1].max(), 1.0)
    X, Y = grid.centers()
    cols = np.clip(np.floor(X).astype(int), 0, 2943)
    rows = np.clip(np.floor(Y).astype(int), 0, 2175)
    valid = (X >= 0) & (X < 2944) & (Y >= 0) & (Y < 2176)
    m = own[rows, cols] & valid
    result = {'mask_pixels_in_frame': int(m.sum())}
    for label, tris in (('native', a), ('rebuilt', b)):
        sil = ~np.isnan(core.raster_surface(grid, tris)) & valid
        inter = int((sil & m).sum())
        union = int((sil | m).sum())
        result[label] = {'silhouette_pixels': int(sil.sum()), 'iou': round(inter / max(union, 1), 4),
                         'mask_covered': round(inter / max(int(m.sum()), 1), 4),
                         'outside_mask_pixels': int((sil & ~m).sum())}
    return result


def geometry_hash(obj):
    payload = [[list(v.co) for v in obj.data.vertices], [list(p.vertices) for p in obj.data.polygons],
               [list(r) for r in obj.matrix_world]]
    return hashlib.sha256(json.dumps(payload).encode()).hexdigest()


def apply_masks(workspace, config, part_ids, write=True):
    """Idempotently write the reviewed MASKS rows for owned nodes."""
    path = Path(config['source_mask_manifest'])
    frozen = json.loads((Path(config['mask_reference']) / 'assignments.json').read_text())
    working = json.loads(path.read_text())
    frozen_rows = {r['source_node']: r for r in frozen['projections']['exterior']['assignments']
                   if 'source_node' in r}
    changes = []
    rows = working['projections']['exterior']['assignments']
    for i, row in enumerate(rows):
        node = row.get('source_node')
        if node not in part_ids:
            continue
        n = int(node.split('-')[-1])
        new = dict(frozen_rows[node])
        if n in MASKS:
            spec = MASKS[n]
            evidence = str(workspace / 'inspection' / f'mask-review-{node}.png')
            if spec.get('include') is not None:
                new = {'reviewed': True, 'source_node': node, 'mask_indices': list(spec['include']),
                       'constraint_kind': 'reviewed-native-silhouette',
                       'native_ownership_reviewed': True,
                       'review_group': 'rocks-terrain-' + node,
                       'review_evidence': evidence,
                       'review_note': spec['note'],
                       'requires_first_hit_gating': True,
                       'composite_envelope': 'native terrain/occluder silhouette shared with neighbours; '
                                             'receiver gating partitions it',
                       'frozen_assignment': {k: frozen_rows[node][k] for k in
                                             ('constraint_kind', 'mask_indices', 'review_group')}}
            exclude = sorted(set(new.get('exclude_mask_indices', [])) | set(spec.get('exclude', [])))
            if exclude:
                new['exclude_mask_indices'] = exclude
                new['exclusions_reviewed'] = True
                new['exclusion_reason'] = spec['reason']
                new['exclusion_evidence'] = evidence
        if row != new:
            rows[i] = new
            changes.append(node)
    if changes and write:
        path.write_text(json.dumps(working, indent=2, sort_keys=True) + '\n')
    return changes, working


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--asset', required=True)
    parser.add_argument('--packet', action='store_true')
    parser.add_argument('--analyse', action='store_true')
    parser.add_argument('--round', default='round-1', choices=['round-1', 'round-2', 'round-3', 'round-4'])
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    workspace = (ROOT / args.round / 'assets' / args.asset).resolve(strict=True)
    config = json.loads((workspace / 'workspace.json').read_text())
    if config['asset_id'] != args.asset:
        raise ValueError('Workspace asset mismatch')
    from render_slots import acquire
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(workspace / 'model.blend'))
    scene = bpy.data.scenes[config['scene_name']]
    bpy.context.window.scene = scene
    collection = bpy.data.collections[config['collection_name']]
    owned = [o for o in collection.all_objects if o.type == 'MESH' and o.get('asset_group') == args.asset]
    part_ids = sorted({o['source_node'] for o in owned})
    if part_ids != config['part_ids'] or len(owned) != len(part_ids):
        raise ValueError(f'Unexpected owned meshes {part_ids}')
    outside = {o.name: geometry_hash(o) for o in scene.objects if o.type == 'MESH' and o not in owned}
    mask_changes, working = apply_masks(workspace, config, part_ids, write=not args.analyse)
    assignments = {r['source_node']: r for r in working['projections']['exterior']['assignments'] if 'source_node' in r}
    masks = MaskStore(config['source_mask_manifest'])
    context = None
    reports = []
    for obj in sorted(owned, key=lambda o: o['source_node']):
        n = int(obj['source_node'].split('-')[-1])
        spec = NODES[n]
        before = geometry_hash(obj)
        neighbours = [o['source_node'] for o in owned if o is not obj
                      and NODES[int(o['source_node'].split('-')[-1])]['mode'] in ('carve', 'trim', 'keep')]
        if spec.get('unhide') and context is None:
            context = NeighbourContext(collection, owned, masks, assignments, set(part_ids))
        report, bm = rebuild(obj, spec, masks, assignments[obj['source_node']], dry=args.analyse,
                             neighbours=neighbours, context=context)
        if bm is not None:
            apply_mesh(obj, bm)
            report['changed'] = True
        report.setdefault('changed', False)
        report['before_geometry_sha256'] = before
        report['geometry_sha256'] = geometry_hash(obj)
        report['vertices'] = len(obj.data.vertices)
        report['faces'] = len(obj.data.polygons)
        obj[TAG] = VERSION
        reports.append(report)
    arch = None
    if set(BRIDGE_ARCH_NODES) <= {o['source_node'] for o in owned} and not args.analyse:
        arch = cut_bridge_arch([o for o in owned if o['source_node'] in BRIDGE_ARCH_NODES])
        for r in reports:
            if r['source_node'] in BRIDGE_ARCH_NODES:
                r['arch_passage'] = arch['results'][r['source_node']]
                r['changed'] = True
                obj = next(o for o in owned if o['source_node'] == r['source_node'])
                r['geometry_sha256'] = geometry_hash(obj)
                r['vertices'] = len(obj.data.vertices)
                r['faces'] = len(obj.data.polygons)
    silhouette = silhouette_stats(owned, assignments, masks)
    after = {o.name: geometry_hash(o) for o in scene.objects if o.type == 'MESH' and o not in owned}
    if after != outside:
        raise ValueError('Recipe changed an outside object')
    result = {'version': 1, 'asset_id': args.asset, 'recipe': str(Path(__file__).resolve()),
              'recipe_version': VERSION, 'recipe_sha256': sha(__file__),
              'core_sha256': sha(HERE / 'rocks_terrain_volumes_core.py'),
              'mask_changes': mask_changes, 'silhouette': silhouette, 'nodes': reports,
              **({'bridge_arch': {k: v for k, v in arch.items() if k != 'results'}} if arch else {}),
              'outside_objects_unchanged': len(outside)}
    inspection = workspace / 'inspection'
    inspection.mkdir(exist_ok=True)
    name = 'recipe-analysis.json' if args.analyse else 'geometry-recipe.json'
    (inspection / name).write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k: v for k, v in result.items() if k != 'nodes'}))
    for r in reports:
        print(json.dumps(r))
    if args.analyse:
        return
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(workspace / 'model.blend'))
    if args.packet:
        sys.path.insert(0, str(TOOLING))
        import refinement_workspace
        packet = refinement_workspace.modified(str(workspace))
        print(json.dumps({'packet': packet.get('status'), 'modified': packet.get('modified')}))


if __name__ == '__main__':
    main()
