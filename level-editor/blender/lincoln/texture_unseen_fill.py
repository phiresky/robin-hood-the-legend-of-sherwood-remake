"""Generated fill for texels no source camera sees (revealed-state caps, back walls, hidden faces).

The earlier texture round filled each asset from its frozen review packet. Texels that remain
neutral in a published worker (state-only objects added after that round, faces changed by later
geometry rounds) have no reviewed packet. This lane renders fixed eight-view packets straight from
the worker, for one explicit target (assets + applied patch set + receiver objects), in the same
two-image Sunburst format, and fills only still-neutral ownership texels of the receivers.

Commands (from the repository root; Blender ones take a Lincoln render slot):

  blender --background --threads 2 --python-exit-code 1 \
    --python level-editor/blender/lincoln/texture_unseen_fill.py -- survey --worker W --output OUT.json
  blender ... -- prepare --worker W --target ID [--target ID ...]
  python3 level-editor/blender/lincoln/texture_unseen_fill.py generate ID... [--prompt-suffix TEXT]
  blender ... -- fill --worker-in W --output DIR [--target ID ...] [--include-unapproved]
  python3 level-editor/blender/lincoln/texture_unseen_fill.py review DIR    # texture-review + gallery
  python3 level-editor/blender/lincoln/texture_unseen_fill.py record-decisions textures/unseen/decisions/batch-N.txt

Targets live in textures/unseen/targets.json: `id`, `assets` (asset groups displayed),
`patches` (applied patch names; [] = covered state), `receivers` (the objects that motivated
the target; they must be displayed, and every displayed object of the target's assets
receives) and optional `prompt_suffix`. An object is displayed for a patch set
when any patch in its `reveal_show_when_applied` is applied and none in
`reveal_hide_when_applied` is.

Packets: 8 orthographic views chosen greedily from 48 candidates (12 azimuths x 4 elevations) to
maximize each unknown texel's best facing cosine; each tile is framed on the receivers' unknown
texels (35% margin). input.png = worker texels, with unknown surfaces replaced by the pure-gray
shading of solid.png (map lighting, ambient + diffuse, cast shadows); mask.png alpha 0 = pixel
shows an unknown texel. views.json binds every displayed object's geometry/UV/slot digest; `fill`
refuses a worker whose displayed objects differ, so packets survive restaging only while geometry
is unchanged.

`fill` writes only texels matching global_reproject.neutral_mask on the receivers: best-facing
first-hit view of the displayed set (cosine > 0.05), sampled from generated-preserved.png inside
the silhouette and inside the editable mask. Geometry, UVs, slots and all other texels are
verified unchanged; combine.json follows texture_combine's schema so verify_texture_combine.py
checks the output. Without --include-unapproved only targets with a recorded texture approval in
textures/unseen/decisions.json (bound to the generated sheet and views) are filled.
"""
import argparse
import ast
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import sys

import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
EDITOR = ROOT / 'level-editor'
WORK = EDITOR / 'work/lincoln-refinement'
UNSEEN = WORK / 'textures/unseen'
TARGETS = UNSEEN / 'targets.json'
DECISIONS = UNSEEN / 'decisions.json'
LIGHTING = WORK / 'lighting-calibration/map-lighting.json'
GENERATION = 'generation-short-no-mask-with-lighting-openrouter'
TILE = 512
SS = 2
AZIMUTHS = range(0, 360, 30)
ELEVATIONS = (15, 35, 55, 75)
SELECT_TILE = 256
SELECT_MIN_FACING = 0.2
SELECT_POINTS = 30000
FRAME_MARGIN = 0.35
FRAME_MIN_FRACTION = 0.6  # minimum tile span, as a fraction of the displayed set's extent
FILL_MIN_COSINE = 0.05
DEPTH_TOLERANCE = 0.02
AUTHORIZATION = {
    'fill': 'User approved AI texture fill for texels the source camera never sees '
            '(relayed by team-lead, 2026-09-26): "yes they should get ai texture fill".',
    'provider': 'OpenRouter openai/gpt-image-2.5-sunburst for Lincoln texture generation '
                '(user: "use openrouter ith sunburst model").',
    'texture_approval': 'pending; nothing is published without the user\'s texture approval',
}


def sha(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            digest.update(block)
    return digest.hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def write(path, data):
    Path(path).write_text(json.dumps(data, indent=2) + '\n')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def targets():
    return {t['id']: t for t in read(TARGETS)['targets']}


def patch_list(value):
    if value is None:
        return None
    if isinstance(value, str):
        return list(ast.literal_eval(value))
    return list(value)


def displayed(obj, patches):
    show = patch_list(obj.get('reveal_show_when_applied'))
    hide = patch_list(obj.get('reveal_hide_when_applied')) or []
    if show is not None and not show:
        raise ValueError('Empty revealed receiver trigger list')
    return (show is None or bool(set(show) & patches)) and not set(hide) & patches


# ---------------------------------------------------------------- cameras

def rotation_for(azimuth, elevation):
    """Blender camera rotation (Euler XYZ = 90-elevation, 0, azimuth); columns are local axes."""
    a, x = math.radians(azimuth), math.radians(90 - elevation)
    rx = np.array([[1, 0, 0], [0, math.cos(x), -math.sin(x)], [0, math.sin(x), math.cos(x)]])
    rz = np.array([[math.cos(a), -math.sin(a), 0], [math.sin(a), math.cos(a), 0], [0, 0, 1]])
    return rz @ rx


class Camera:
    """Orthographic square tile; project() -> (tile x, top-origin tile y, depth toward camera)."""

    def __init__(self, azimuth, elevation, frame_points, bound_points, tile):
        self.azimuth, self.elevation, self.tile = azimuth, elevation, tile
        self.rotation = rotation_for(azimuth, elevation)
        self.toward = self.rotation[:, 2]
        local = frame_points @ self.rotation
        low, high = local[:, :2].min(0), local[:, :2].max(0)
        size = np.maximum(high - low, 1.0)
        low, high = low - size * FRAME_MARGIN, high + size * FRAME_MARGIN
        bound = bound_points @ self.rotation
        # Keep enough of the displayed asset in frame for the model to see its materials.
        extent = bound[:, :2].max(0) - bound[:, :2].min(0)
        grow = np.maximum(0, extent * FRAME_MIN_FRACTION - (high - low)) / 2
        low, high = low - grow, high + grow
        low = np.maximum(low, bound[:, :2].min(0))
        high = np.minimum(high, bound[:, :2].max(0))
        center = (low + high) / 2
        self.scale = float(max(high - low) * 1.02)
        depth = float(bound[:, 2].max()) + 100.0
        self.matrix = np.eye(4)
        self.matrix[:3, :3] = self.rotation
        self.matrix[:3, 3] = self.rotation @ np.array([center[0], center[1], depth])

    def project(self, points):
        local = (points - self.matrix[:3, 3]) @ self.rotation
        x = (.5 + local[..., 0] / self.scale) * self.tile
        y = (.5 - local[..., 1] / self.scale) * self.tile
        return x, y, local[..., 2]

    def record(self, index, left, top):
        return {'index': index, 'azimuth_degrees': self.azimuth, 'elevation_degrees': self.elevation,
                'camera_matrix_world': self.matrix.tolist(), 'ortho_scale': self.scale,
                'crop': {'left': left, 'top': top, 'width': self.tile, 'height': self.tile}}


def raster(camera, corners, ss):
    """Front-most triangle per subsample: depth (camera z, larger = nearer) and triangle index."""
    size = camera.tile * ss
    x, y, d = camera.project(corners.reshape(-1, 3))
    xs, ys, d = (x * ss).reshape(-1, 3), (y * ss).reshape(-1, 3), d.reshape(-1, 3)
    depth = np.full((size, size), -np.inf)
    index = np.full((size, size), -1, dtype=np.int64)
    area = (xs[:, 1] - xs[:, 0]) * (ys[:, 2] - ys[:, 0]) - (xs[:, 2] - xs[:, 0]) * (ys[:, 1] - ys[:, 0])
    x0 = np.maximum(0, np.ceil(xs.min(1) - .5)).astype(int)
    x1 = np.minimum(size - 1, np.floor(xs.max(1) - .5)).astype(int)
    y0 = np.maximum(0, np.ceil(ys.min(1) - .5)).astype(int)
    y1 = np.minimum(size - 1, np.floor(ys.max(1) - .5)).astype(int)
    for t in np.flatnonzero((np.abs(area) > 1e-12) & (x1 >= x0) & (y1 >= y0)):
        gx = np.arange(x0[t], x1[t] + 1) + .5
        gy = (np.arange(y0[t], y1[t] + 1) + .5)[:, None]
        (ax, bx, cx), (ay, by, cy) = xs[t], ys[t]
        w0 = ((bx - gx) * (cy - gy) - (cx - gx) * (by - gy)) / area[t]
        w1 = ((cx - gx) * (ay - gy) - (ax - gx) * (cy - gy)) / area[t]
        w2 = 1 - w0 - w1
        inside = (w0 >= -1e-9) & (w1 >= -1e-9) & (w2 >= -1e-9)
        z = w0 * d[t, 0] + w1 * d[t, 1] + w2 * d[t, 2]
        window = depth[y0[t]:y1[t] + 1, x0[t]:x1[t] + 1]
        take = inside & (z > window)
        window[take] = z[take]
        index[y0[t]:y1[t] + 1, x0[t]:x1[t] + 1][take] = t
    return depth, index, (xs, ys)


def visible(camera, depth, ss, points):
    x, y, d = camera.project(points)
    fx, fy = x * ss - .5, y * ss - .5
    size = depth.shape[0]
    result = np.zeros(len(points), dtype=bool)
    for ox in (0, 1):
        for oy in (0, 1):
            cx, cy = np.floor(fx).astype(int) + ox, np.floor(fy).astype(int) + oy
            inside = (cx >= 0) & (cx < size) & (cy >= 0) & (cy < size)
            front = np.where(inside, depth[np.clip(cy, 0, size - 1), np.clip(cx, 0, size - 1)], -np.inf)
            result |= inside & (front <= d + DEPTH_TOLERANCE)
    return result, x, y


# ---------------------------------------------------------------- worker scene

def object_digest(record, scene, gr):
    obj = record['object']
    digest = hashlib.sha256(obj.name.encode())
    digest.update(np.ascontiguousarray(record['corners']).tobytes())
    digest.update(record['slots'].tobytes())
    for layer in obj.data.uv_layers:
        digest.update(layer.name.encode() + gr.slot_uvs(obj, layer.name).tobytes())
    return digest.hexdigest()


def material_color(material):
    if material is None:
        return np.array([128, 128, 128])
    if material.node_tree:
        for node in material.node_tree.nodes:
            if node.type == 'BSDF_PRINCIPLED':
                return np.rint(np.array(node.inputs['Base Color'].default_value[:3]) ** (1 / 2.2) * 255)
    return np.rint(np.array(material.diffuse_color[:3]) ** (1 / 2.2) * 255)


def fillable(scene, obj, slot):
    """(binding, kind) of a slot whose unknown texels may be filled, else None.

    kind 'ownership': per-asset exterior atlases, unknown = global_reproject.neutral_mask.
    kind 'ground': the terrain source-projection atlas (one texel per source pixel),
    unknown = alpha 0 (RGB 128); observed texels have alpha 255.
    """
    binding = scene.slot_binding(obj, slot)
    if binding is None:
        return None
    if obj.get('source_node') == 'ground':
        return binding, 'ground'
    if binding['kind'] == 'ownership':
        return binding, 'ownership'
    return None


def unknown_texels_mask(kind, texels, normals, face_normals, gr):
    """Unknown flags for (N, 4) uint8 texels of a fillable atlas."""
    if kind == 'ground':
        return texels[:, 3] == 0
    gray = (texels[:, 0] == texels[:, 1]) & (texels[:, 1] == texels[:, 2]) & (texels[:, 3] == 255)
    expected = np.stack([gr.neutral_value(normals), gr.neutral_value(face_normals)]) * 255
    return gray & (np.abs(texels[:, 0].astype(float) - expected).min(0) <= 1.0)


def island_unknown(kind, atlas, rows, cols, normals, face_normal, gr):
    if kind == 'ground':
        return atlas[rows, cols, 3] == 0
    return gr.neutral_mask(atlas, rows, cols, normals, face_normal)


def island_receivers(kind, unknown, interior):
    # Terrain uses a continuous atlas: a face's gutter can belong to a different
    # face. Extending its edge samples there paints stripes across that neighbour.
    return unknown & interior if kind == 'ground' else unknown


def asset_of(obj):
    """Catalog asset of a worker mesh; staged terrain meshes carry only source_node 'ground'."""
    return obj.get('asset_group') or ('lincoln-terrain' if obj.get('source_node') == 'ground' else None)


def subset(record, keep):
    """A mesh record restricted to the kept triangles (face normals stay indexed by polygon)."""
    return dict(record, corners=record['corners'][keep], loops=record['loops'][keep],
                polygons=record['polygons'][keep], slots=record['slots'][keep], normals=record['normals'][keep])


class Target:
    """Displayed triangles of one target state, their texture bindings and receiver records.

    Optional spec keys: `context_assets` are displayed (visibility, framing context) but never
    receive; `region` {"x": [lo, hi], "y": [lo, hi]} keeps only terrain ('ground') triangles whose
    centroid lies inside it.
    """

    def __init__(self, spec, scene, gr):
        self.spec, self.gr, self.scene = spec, gr, scene
        patches = set(spec['patches'])
        region = spec.get('region')
        receiving = set(spec['assets'])
        shown = receiving | set(spec.get('context_assets', []))
        self.records = []
        for record in scene.meshes:
            obj = record['object']
            if asset_of(obj) not in shown or not displayed(obj, patches):
                continue
            if region and obj.get('source_node') == 'ground':
                # Ground polygons can be very large n-gons: keep triangles overlapping the region;
                # texels are restricted to the region separately (in_region).
                low, high = record['corners'].min(1), record['corners'].max(1)
                keep = ((high[:, 0] >= region['x'][0]) & (low[:, 0] <= region['x'][1])
                        & (high[:, 1] >= region['y'][0]) & (low[:, 1] <= region['y'][1]))
                if not keep.any():
                    continue
                record = subset(record, keep)
            self.records.append(record)
        names = {r['object'].name for r in self.records}
        missing = set(spec['receivers']) - names
        require(not missing, f"{spec['id']}: receivers not displayed in this state: {sorted(missing)}")
        # Every displayed object of the target's own assets receives: the input marks their
        # unknown texels editable, and in a revealed state appearance copies and unchanged parts
        # (e.g. room floors under a removed roof) show unseen faces besides the named cut objects.
        # `receivers` are the objects that motivated the target and must be displayed.
        self.receivers = [r for r in self.records if asset_of(r['object']) in receiving]
        self.region = region
        self.corners = np.concatenate([r['corners'] for r in self.records])
        self.normals = np.concatenate([r['normals'] for r in self.records])
        self.face_normals = np.concatenate([r['face_normals'][r['polygons']] for r in self.records])
        self.tri_ground = np.concatenate([np.full(len(r['corners']), r['object'].get('source_node') == 'ground')
                                          for r in self.records])
        receiver_ids = {id(r) for r in self.receivers}
        self.receiver_tri = np.concatenate([np.full(len(r['corners']), id(r) in receiver_ids) for r in self.records])
        # Per triangle texture binding: image id (-1 = flat color), UV corners, fill kind.
        self.images, self.image_kind = [], []
        image_ids = {}
        tri_image, tri_uv, tri_color = [], [], []
        for record in self.records:
            obj = record['object']
            count = len(record['corners'])
            image = np.full(count, -1)
            uv = np.zeros((count, 3, 2))
            color = np.zeros((count, 3))
            for slot in np.unique(record['slots']):
                chosen = record['slots'] == slot
                binding = scene.slot_binding(obj, int(slot))
                if binding is None:
                    material = obj.data.materials[slot] if slot < len(obj.data.materials) else None
                    color[chosen] = material_color(material)
                    continue
                entry = fillable(scene, obj, int(slot))
                key = binding['image'].name
                if key not in image_ids:
                    image_ids[key] = len(self.images)
                    self.images.append(binding['image'])
                    self.image_kind.append(entry[1] if entry else None)
                image[chosen] = image_ids[key]
                uv[chosen] = gr.slot_uvs(obj, binding['uv'])[record['loops'][chosen]]
            tri_image.append(image)
            tri_uv.append(uv)
            tri_color.append(color)
        self.tri_image = np.concatenate(tri_image)
        self.tri_uv = np.concatenate(tri_uv)
        self.tri_color = np.concatenate(tri_color)
        self._atlases = {}

    def in_region(self, obj, points):
        """Texels of a region-limited ground mesh outside the region are never unknown targets."""
        if not self.region or obj.get('source_node') != 'ground':
            return np.ones(len(points), dtype=bool)
        r = self.region
        return ((points[:, 0] >= r['x'][0]) & (points[:, 0] <= r['x'][1])
                & (points[:, 1] >= r['y'][0]) & (points[:, 1] <= r['y'][1]))

    def bound(self):
        """Framing bound: displayed corners, with region-limited ground clipped to its region."""
        parts = []
        for record in self.records:
            corners = record['corners'].reshape(-1, 3).copy()
            if self.region and record['object'].get('source_node') == 'ground':
                corners[:, 0] = corners[:, 0].clip(*self.region['x'])
                corners[:, 1] = corners[:, 1].clip(*self.region['y'])
            parts.append(corners)
        return np.concatenate(parts)

    def atlas(self, image_id):
        if image_id not in self._atlases:
            self._atlases[image_id] = self.gr.read_image(self.images[image_id])
        return self._atlases[image_id]

    def digests(self):
        return {r['object'].name: object_digest(r, self.scene, self.gr) for r in self.records}

    def unknown_texels(self):
        """Neutral receiver texels: positions and normals (island interiors and gutters)."""
        gr, positions, normals, interior = self.gr, [], [], []
        for record in self.receivers:
            obj = record['object']
            for slot in sorted(set(record['slots'].tolist())):
                entry = fillable(self.scene, obj, slot)
                if entry is None:
                    continue
                binding, kind = entry
                atlas = gr.read_image(binding['image'])
                uv = gr.slot_uvs(obj, binding['uv'])
                for face, rows, cols, points, face_normals, inner in gr.islands(
                        record, uv, binding['image'].size, lambda group, s=slot: record['slots'][group[0]] == s):
                    neutral = island_unknown(kind, atlas, rows, cols, face_normals, record['face_normals'][face], gr)
                    neutral = island_receivers(kind, neutral, inner)
                    neutral &= self.in_region(obj, points)
                    positions.append(points[neutral])
                    normals.append(face_normals[neutral])
                    interior.append(inner[neutral])
        if not positions:
            return np.zeros((0, 3)), np.zeros((0, 3)), np.zeros(0, dtype=bool)
        return np.concatenate(positions), np.concatenate(normals), np.concatenate(interior)

    def shade(self, camera, index, coords, lighting, bvh):
        """Render one tile: texel colors, unknown flags, pure-gray shading (top-origin rows)."""
        from mathutils import Vector
        xs, ys = coords
        size = index.shape[0]
        covered = index >= 0
        t = index[covered]
        gy, gx = np.nonzero(covered)
        px, py = gx + .5, gy + .5
        (ax, bx, cx), (ay, by, cy) = xs[t].T, ys[t].T
        area = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay)
        w0 = ((bx - px) * (cy - py) - (cx - px) * (by - py)) / area
        w1 = ((cx - px) * (ay - py) - (ax - px) * (cy - py)) / area
        weights = np.stack([w0, w1, 1 - w0 - w1], axis=1).clip(0, 1)
        weights /= weights.sum(1, keepdims=True)
        world = np.einsum('nk,nkj->nj', weights, self.corners[t])
        uv = np.einsum('nk,nkj->nj', weights, self.tri_uv[t])
        rgb = np.zeros((len(t), 3))
        unknown = np.zeros(len(t), dtype=bool)
        ground_unknown = np.zeros(len(t), dtype=bool)
        images = self.tri_image[t]
        flat = images < 0
        rgb[flat] = self.tri_color[t[flat]]
        for image_id in np.unique(images[~flat]):
            chosen = images == image_id
            atlas = self.atlas(int(image_id))
            height, width = atlas.shape[:2]
            col = np.clip(np.floor(uv[chosen, 0] * width).astype(int), 0, width - 1)
            row = np.clip(np.floor(uv[chosen, 1] * height).astype(int), 0, height - 1)
            texel = atlas[row, col]
            rgb[chosen] = texel[:, :3]
            kind = self.image_kind[image_id]
            if kind == 'ground':
                ground_unknown[chosen] = texel[:, 3] == 0
            if kind:
                tri = t[chosen]
                # Context objects never receive, so their unknown texels are not editable.
                unknown[chosen] = self.receiver_tri[tri] & unknown_texels_mask(
                    kind, texel, self.normals[tri], self.face_normals[tri], self.gr)
        if self.region:
            r = self.region
            inside = ((world[:, 0] >= r['x'][0]) & (world[:, 0] <= r['x'][1])
                      & (world[:, 1] >= r['y'][0]) & (world[:, 1] <= r['y'][1]))
            unknown &= ~self.tri_ground[t] | inside
            # Unknown ground outside the region is neither shown nor editable (background).
            keep = ~(self.tri_ground[t] & ~inside & ground_unknown)
            t, gy, gx, world, rgb, unknown = t[keep], gy[keep], gx[keep], world[keep], rgb[keep], unknown[keep]
        # Pure-gray lighting: normals turned toward the camera, cast shadows toward the sun.
        normal = self.normals[t] * np.where(self.normals[t] @ camera.toward < 0, -1, 1)[:, None]
        sun = np.array(lighting['toward_sun'])
        lambert = np.maximum(0, normal @ sun)
        lit = np.flatnonzero(lambert > 0)
        direction = Vector(sun)
        for i in lit:
            origin = Vector(world[i] + normal[i] * lighting['shadow_epsilon'] * 10)
            if bvh.ray_cast(origin, direction)[0] is not None:
                lambert[i] = 0
        gray = np.clip((lighting['ambient'] + lighting['diffuse'] * lambert) * 255, 0, 255)
        tile = self.tile_arrays(size, gy, gx, rgb, unknown, gray, ss=size // camera.tile)
        return tile

    @staticmethod
    def tile_arrays(size, gy, gx, rgb, unknown, gray, ss):
        """Downsample subsamples: color mean of covered, unknown if any subsample unknown."""
        n = size // ss
        count = np.zeros((size, size))
        color = np.zeros((size, size, 3))
        flag = np.zeros((size, size), dtype=bool)
        shade = np.zeros((size, size))
        count[gy, gx] = 1
        color[gy, gx] = rgb
        flag[gy, gx] = unknown
        shade[gy, gx] = gray

        def pool(a):
            return a.reshape(n, ss, n, ss, *a.shape[2:]).sum(axis=(1, 3))
        hits = pool(count)
        covered = hits > 0
        safe = np.maximum(hits, 1)
        mean = pool(color) / safe[..., None]
        solid = pool(shade) / safe
        editable = pool(flag.astype(float)) > 0
        return covered, mean, editable, solid


def bvh_for(target):
    from mathutils.bvhtree import BVHTree
    corners = target.corners.reshape(-1, 3)
    return BVHTree.FromPolygons([tuple(p) for p in corners],
                                [(3 * i, 3 * i + 1, 3 * i + 2) for i in range(len(target.corners))],
                                all_triangles=True)


def full_scene(gr):
    """All render-visible working meshes, revealed-state objects included.

    global_reproject.Scene keeps covered-state geometry only (its source camera never sees state
    objects); targets here select their state themselves through `displayed`.
    """
    covered = gr.covered_state_mesh
    gr.covered_state_mesh = lambda properties: True
    try:
        return gr.Scene()
    finally:
        gr.covered_state_mesh = covered


def open_worker(path):
    import bpy
    sys.path.insert(0, str(HERE))
    sys.path.insert(0, str(EDITOR / 'refinement/blender'))
    from render_slots import acquire
    import global_reproject as gr
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(path))
    bpy.context.window.scene = bpy.data.scenes[gr.SCENE]
    return gr, full_scene(gr)


# ---------------------------------------------------------------- survey

def survey(worker, output):
    """Neutral ownership texels per object (interior = inside the UV island, not gutter)."""
    worker = Path(worker).resolve(strict=True)
    gr, scene = open_worker(worker)
    rows = []
    for record in scene.meshes:
        obj = record['object']
        counts = {'neutral': 0, 'neutral_interior': 0, 'island_interior': 0}
        for slot in sorted(set(record['slots'].tolist())):
            binding = scene.slot_binding(obj, slot)
            if binding is None or binding['kind'] != 'ownership':
                continue
            atlas = gr.read_image(binding['image'])
            uv = gr.slot_uvs(obj, binding['uv'])
            for face, rows_, cols, _, normals, inner in gr.islands(
                    record, uv, binding['image'].size, lambda group, s=slot: record['slots'][group[0]] == s):
                neutral = gr.neutral_mask(atlas, rows_, cols, normals, record['face_normals'][face])
                counts['neutral'] += int(neutral.sum())
                counts['neutral_interior'] += int((neutral & inner).sum())
                counts['island_interior'] += int(inner.sum())
        rows.append({'object': obj.name, 'asset_group': obj.get('asset_group'),
                     'state_recipe': obj.get('state_recipe'),
                     'reveal_show_when_applied': patch_list(obj.get('reveal_show_when_applied')),
                     'reveal_hide_when_applied': patch_list(obj.get('reveal_hide_when_applied')), **counts})
        print(json.dumps({'object': obj.name, **counts}), flush=True)
    assets = {}
    for row in rows:
        entry = assets.setdefault(row['asset_group'], {'neutral_interior': 0, 'island_interior': 0,
                                                       'state_neutral_interior': 0})
        entry['neutral_interior'] += row['neutral_interior']
        entry['island_interior'] += row['island_interior']
        if row['state_recipe']:
            entry['state_neutral_interior'] += row['neutral_interior']
    write(output, {'version': 1, 'worker': str(worker), 'worker_sha256': sha(worker),
                   'rule': 'global_reproject.neutral_mask over ownership islands; interior = inside the UV triangle',
                   'assets': dict(sorted(assets.items(), key=lambda kv: -kv[1]['neutral_interior'])),
                   'objects': rows})


def survey_visible(worker, output, tile=4096, min_facing=0.2):
    """Neutral ownership texels a map viewer can actually see in the covered state.

    Whole-map orthographic views (8 azimuths x elevations 30/50, plus the source direction) of
    every mesh displayed with no patch applied; a neutral island texel counts when it is first-hit
    visible with facing cosine > min_facing in at least one view. Hidden faces (undersides,
    faces inside volumes or under terrain) are excluded, unlike the raw `survey` count.
    """
    worker = Path(worker).resolve(strict=True)
    gr, scene = open_worker(worker)
    records = [r for r in scene.meshes if displayed(r['object'], set())]
    corners = np.concatenate([r['corners'] for r in records])
    bound = corners.reshape(-1, 3)
    cameras = [Camera(0, 35, bound, bound, tile)]
    cameras += [Camera(a, e, bound, bound, tile) for e in (30, 50) for a in range(0, 360, 45)]
    depths = []
    for camera in cameras:
        depths.append(raster(camera, corners, 1)[0])
        print(json.dumps({'view': [camera.azimuth, camera.elevation], 'pixel_size': camera.scale / tile}), flush=True)
    rows = []
    for record in records:
        obj = record['object']
        counts = {'neutral_interior': 0, 'visible_neutral_interior': 0, 'island_interior': 0}
        for slot in sorted(set(record['slots'].tolist())):
            binding = scene.slot_binding(obj, slot)
            if binding is None or binding['kind'] != 'ownership':
                continue
            atlas = gr.read_image(binding['image'])
            uv = gr.slot_uvs(obj, binding['uv'])
            for face, rows_, cols, points, normals, inner in gr.islands(
                    record, uv, binding['image'].size, lambda group, s=slot: record['slots'][group[0]] == s):
                counts['island_interior'] += int(inner.sum())
                neutral = gr.neutral_mask(atlas, rows_, cols, normals, record['face_normals'][face]) & inner
                if not neutral.any():
                    continue
                counts['neutral_interior'] += int(neutral.sum())
                points, normals = points[neutral], normals[neutral]
                seen = np.zeros(len(points), dtype=bool)
                for camera, depth in zip(cameras, depths):
                    facing = normals @ camera.toward > min_facing
                    if facing.any():
                        seen[facing] |= visible(camera, depth, 1, points[facing])[0]
                counts['visible_neutral_interior'] += int(seen.sum())
        rows.append({'object': obj.name, 'asset_group': obj.get('asset_group'), **counts})
    assets = {}
    for row in rows:
        entry = assets.setdefault(row['asset_group'], {'visible_neutral_interior': 0, 'neutral_interior': 0,
                                                       'island_interior': 0, 'objects': {}})
        for key in ('visible_neutral_interior', 'neutral_interior', 'island_interior'):
            entry[key] += row[key]
        if row['visible_neutral_interior']:
            entry['objects'][row['object']] = row['visible_neutral_interior']
    write(output, {'version': 1, 'worker': str(worker), 'worker_sha256': sha(worker), 'state': 'covered',
                   'views': [[c.azimuth, c.elevation] for c in cameras], 'tile': tile, 'min_facing': min_facing,
                   'assets': dict(sorted(assets.items(), key=lambda kv: -kv[1]['visible_neutral_interior']))})


# ---------------------------------------------------------------- prepare

def select_cameras(target, points, normals, *, required_views=()):
    if len(points) > SELECT_POINTS:
        pick = np.random.default_rng(0).choice(len(points), SELECT_POINTS, replace=False)
        points, normals = points[pick], normals[pick]
    bound = target.bound()
    candidates, facing = [], []
    for elevation in ELEVATIONS:
        for azimuth in AZIMUTHS:
            camera = Camera(azimuth, elevation, points, bound, SELECT_TILE)
            depth, _, _ = raster(camera, target.corners, 1)
            seen, _, _ = visible(camera, depth, 1, points)
            score = normals @ camera.toward
            candidates.append((azimuth, elevation))
            facing.append(np.where(seen & (score > SELECT_MIN_FACING), score, 0))
    facing = np.array(facing)
    best = np.zeros(len(points))
    required = [tuple(view) for view in required_views]
    if len(required) > 8 or len(set(required)) != len(required):
        raise ValueError('Required camera views must be unique and fit the eight-view sheet')
    if any(view not in candidates for view in required):
        raise ValueError('Required camera view is outside the candidate grid')
    chosen = [candidates.index(view) for view in required]
    for index in chosen:
        best = np.maximum(best, facing[index])
    for _ in range(8-len(chosen)):
        gain = np.maximum(facing, best).sum(1) - best.sum()
        gain[chosen] = -1
        pick = int(np.argmax(gain))
        chosen.append(pick)
        best = np.maximum(best, facing[pick])
    coverage = float((best > 0).mean()) if len(best) else 1.0
    required_indices = [candidates.index(view) for view in required]
    order = required_indices + sorted((i for i in chosen if i not in required_indices),
                                      key=lambda i: (candidates[i][0], candidates[i][1]))
    return [candidates[i] for i in order], coverage


def prepare(worker, ids):
    from PIL import Image
    worker = Path(worker).resolve(strict=True)
    worker_sha = sha(worker)
    gr, scene = open_worker(worker)
    lighting_config = read(LIGHTING)
    lighting = lighting_config['lighting']
    specs = targets()
    for target_id in ids:
        spec = specs[target_id]
        output = UNSEEN / target_id
        require(not output.exists(), 'Experiment already exists: ' + str(output))
        target = Target(spec, scene, gr)
        points, normals, interior = target.unknown_texels()
        require(len(points), target_id + ': receivers hold no neutral texels')
        frame = points[interior] if interior.any() else points
        views, coverage = select_cameras(target, frame, normals[interior] if interior.any() else normals)
        bvh = bvh_for(target)
        width, height = 4 * TILE, 2 * TILE
        input_sheet = np.zeros((height, width, 4), dtype=np.uint8)
        solid_sheet = np.zeros((height, width, 4), dtype=np.uint8)
        mask_sheet = np.full((height, width, 4), 255, dtype=np.uint8)
        records, counts = [], {}
        (output / 'views').mkdir(parents=True)
        for index, (azimuth, elevation) in enumerate(views):
            camera = Camera(azimuth, elevation, frame, target.bound(), TILE)
            _, tri, coords = raster(camera, target.corners, SS)
            covered, color, editable, gray = target.shade(camera, tri, coords, lighting, bvh)
            editable &= covered
            tile_input = np.zeros((TILE, TILE, 4), dtype=np.uint8)
            tile_solid = np.zeros((TILE, TILE, 4), dtype=np.uint8)
            tile_mask = np.full((TILE, TILE, 4), 255, dtype=np.uint8)
            gray8 = np.rint(gray).astype(np.uint8)
            tile_solid[covered] = np.stack([gray8[covered]] * 3 + [np.full(covered.sum(), 255, np.uint8)], 1)
            tile_input[covered, :3] = np.rint(color[covered]).astype(np.uint8)
            tile_input[covered, 3] = 255
            tile_input[editable, :3] = tile_solid[editable, :3]
            tile_mask[editable, 3] = 0
            left, top = index % 4 * TILE, index // 4 * TILE
            input_sheet[top:top + TILE, left:left + TILE] = tile_input
            solid_sheet[top:top + TILE, left:left + TILE] = tile_solid
            mask_sheet[top:top + TILE, left:left + TILE] = tile_mask
            for name, tile in (('input', tile_input), ('mask', tile_mask), ('solid', tile_solid)):
                Image.fromarray(tile).save(output / f'views/view-{index}-{name}.png')
            record = camera.record(index, left, top)
            record.update(input=f'views/view-{index}-input.png', mask=f'views/view-{index}-mask.png',
                          counts={'covered': int(covered.sum()), 'unknown': int(editable.sum())})
            records.append(record)
            print(json.dumps({'target': target_id, 'view': index, 'azimuth': azimuth,
                              'elevation': elevation, **record['counts']}), flush=True)
        Image.fromarray(input_sheet).save(output / 'input.png')
        Image.fromarray(solid_sheet).save(output / 'solid.png')
        Image.fromarray(mask_sheet).save(output / 'mask.png')
        manifest = {
            'version': 1, 'kind': 'lincoln-unseen-fill', 'target': spec, 'asset_id': target_id,
            'worker': str(worker), 'worker_sha256': worker_sha,
            'display_objects': target.digests(), 'receivers': spec['receivers'],
            'receiver_unknown_texels': int(len(points)), 'receiver_unknown_interior_texels': int(interior.sum()),
            'camera_selection': {'candidates': [list(AZIMUTHS), list(ELEVATIONS)],
                                 'min_facing': SELECT_MIN_FACING,
                                 'interior_unknown_covered_fraction': coverage},
            'framing': {'margin': FRAME_MARGIN, 'min_fraction_of_displayed_extent': FRAME_MIN_FRACTION},
            'lighting': lighting, 'lighting_config_sha256': sha(LIGHTING),
            'source_image': None, 'tile_size': [TILE, TILE],
            'layout': {'columns': 4, 'rows': 2, 'width': width, 'height': height}, 'views': records,
            'rule': ('input = worker texels (nearest, 2x2 supersampled); pixels showing any neutral receiver or '
                     'display texel take the pure-gray solid shading and are editable (mask alpha 0).'),
        }
        write(output / 'views.json', manifest)
        revision = hashlib.sha256(json.dumps({'worker_sha256': worker_sha, 'display': manifest['display_objects'],
                                              'target': spec}, sort_keys=True).encode()).hexdigest()
        approval = {'status': 'approved', 'approved_by': 'user', 'asset_id': target_id,
                    'scope': 'AI fill of source-unseen texels; texture candidate generation only',
                    'exact_user_text': 'yes they should get ai texture fill',
                    'relayed_by': 'team-lead, 2026-09-26', 'authorization': AUTHORIZATION,
                    'geometry_revision': revision, 'input_sha256': sha(output / 'input.png'),
                    'solid_sha256': sha(output / 'solid.png'), 'mask_sha256': sha(output / 'mask.png'),
                    'texture_approval': 'pending'}
        write(output / 'approval.json', approval)
        write(output / 'preparation.json', {'files': {str(p.relative_to(output)): sha(p) for p in sorted(output.rglob('*'))
                                                      if p.is_file()}})
        print(json.dumps({'target': target_id, 'unknown_texels': int(len(points)),
                          'interior': int(interior.sum()), 'coverage': coverage, 'views': views}), flush=True)


def verify_prepared(experiment):
    for name, digest in read(experiment / 'preparation.json')['files'].items():
        require(sha(experiment / name) == digest, 'Prepared artifact changed: ' + name)


# ---------------------------------------------------------------- generate

def generate(ids, prompt_suffix=None):
    for target_id in ids:
        experiment = UNSEEN / target_id
        verify_prepared(experiment)
        suffix = prompt_suffix or targets()[target_id].get('prompt_suffix')
        command = ['node', str(EDITOR / 'pipeline/src/refinement/generate-textures.ts'), str(experiment),
                   '--generate', '--prompt-variant', 'short', '--no-mask',
                   '--lighting-reference', str(experiment / 'solid.png'), '--provider', 'openrouter']
        if suffix:
            command += ['--prompt-suffix', suffix]
        env = dict(os.environ, NODE_USE_ENV_PROXY='1')  # Node's fetch ignores HTTP(S)_PROXY otherwise.
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, env=env)
        (experiment / 'generation-log-openrouter.txt').write_text(result.stdout + result.stderr)
        if result.returncode:
            print(json.dumps({'target': target_id, 'status': 'failed', 'reason': result.stderr.strip()[-1500:]}), flush=True)
            continue
        generation = experiment / GENERATION
        report = read(generation / 'generation.json')
        write(generation / 'generation-provenance.json',
              {'version': 1, 'provider': 'openrouter', 'command': command[2:], 'prompt_suffix': suffix,
               'authorization': AUTHORIZATION, 'generation_sha256': sha(generation / 'generation.json')})
        print(json.dumps({'target': target_id, 'status': 'generated', 'filled': report['filled'],
                          'changedProtected': report['changedProtected']}), flush=True)


# ---------------------------------------------------------------- fill

def approved(target_id, experiment):
    if not DECISIONS.exists():
        return False
    bound = {'generated_preserved_sha256': sha(experiment / GENERATION / 'generated-preserved.png'),
             'views_sha256': sha(experiment / 'views.json')}
    return any(d['asset_id'] == target_id and d.get('scope') == 'texture' and d.get('decision') == 'approved'
               and all(d['evidence_sha256'].get(k) == v for k, v in bound.items())
               for d in read(DECISIONS)['decisions'])


def fill_target(target_id, scene, gr):
    from PIL import Image
    sys.path.insert(0, str(EDITOR / 'refinement/blender'))
    import texture_combine as tc
    experiment = UNSEEN / target_id
    verify_prepared(experiment)
    manifest = read(experiment / 'views.json')
    generation = experiment / GENERATION
    report = read(generation / 'generation.json')
    require(report.get('changedProtected') == 0, target_id + ': generated sheet changed protected pixels')
    target = Target(manifest['target'], scene, gr)
    digests = target.digests()
    require(digests == manifest['display_objects'],
            f'{target_id}: displayed geometry differs from the prepared packet: '
            f'{sorted(k for k in set(digests) | set(manifest["display_objects"]) if digests.get(k) != manifest["display_objects"].get(k))[:8]}')
    sheet = np.asarray(Image.open(generation / 'generated-preserved.png').convert('RGBA')).astype(np.float64)
    solid = np.asarray(Image.open(experiment / 'solid.png').convert('RGBA'))[..., 3] > 0
    editable = np.asarray(Image.open(experiment / 'mask.png').convert('RGBA'))[..., 3] == 0
    cameras, depths = [], []
    for entry in manifest['views']:
        camera = Camera(entry['azimuth_degrees'], entry['elevation_degrees'],
                        np.zeros((1, 3)), np.zeros((1, 3)), TILE)
        camera.matrix = np.array(entry['camera_matrix_world'])
        camera.rotation = camera.matrix[:3, :3]
        camera.toward = camera.rotation[:, 2]
        camera.scale = entry['ortho_scale']
        camera.left, camera.top = entry['crop']['left'], entry['crop']['top']
        depth, _, _ = raster(camera, target.corners, SS)
        cameras.append(camera)
        depths.append(depth)
    objects = {}
    for record in target.receivers:
        obj = record['object']
        counts = {'neutral_before': 0, 'generated': 0, 'unseen': 0}
        for slot in sorted(set(record['slots'].tolist())):
            entry = fillable(scene, obj, slot)
            if entry is None:
                continue
            binding, kind = entry
            image = binding['image']
            atlas = gr.read_image(image)
            before = atlas.copy()
            uv = gr.slot_uvs(obj, binding['uv'])
            for face, rows, cols, positions, normals, inner in gr.islands(
                    record, uv, image.size, lambda group, s=slot: record['slots'][group[0]] == s):
                neutral = island_unknown(kind, atlas, rows, cols, normals, record['face_normals'][face], gr)
                neutral = island_receivers(kind, neutral, inner)
                neutral &= target.in_region(obj, positions)
                if not neutral.any():
                    continue
                rows, cols, positions, normals = rows[neutral], cols[neutral], positions[neutral], normals[neutral]
                counts['neutral_before'] += len(rows)
                best = np.full(len(rows), -np.inf)
                colors = np.zeros((len(rows), 3), dtype=np.uint8)
                # Front-facing views first. Single-sided cut walls and card backs show their
                # reverse side in the displayed state; texels no front view samples then take the
                # best back-facing view (both sides share one texel; counted separately).
                for side in (1, -1):
                    pending = ~np.isfinite(best)
                    front = best.copy()
                    for camera, depth in zip(cameras, depths):
                        score = side * (normals @ camera.toward)
                        candidate = pending & (score > FILL_MIN_COSINE) & (score > best)
                        if not candidate.any():
                            continue
                        index = np.flatnonzero(candidate)
                        seen, x, y = visible(camera, depth, SS, positions[index])
                        x, y = x + camera.left, y + camera.top
                        view = type('View', (), {'crop': {'left': camera.left, 'top': camera.top,
                                                          'width': TILE, 'height': TILE}})
                        rgb, inside = tc.sample(sheet, solid, view, x, y, editable, np.ones(len(index), dtype=bool))
                        take = seen & inside
                        colors[index[take]] = rgb[take]
                        best[index[take]] = score[index[take]]
                    if side == -1:
                        counts['generated_back_facing'] = counts.get('generated_back_facing', 0) + int(
                            (np.isfinite(best) & ~np.isfinite(front)).sum())
                filled = np.isfinite(best)
                counts['generated'] += int(filled.sum())
                counts['unseen'] += int((~filled).sum())
                if filled.any():
                    atlas[rows[filled], cols[filled], :3] = colors[filled]
                    atlas[rows[filled], cols[filled], 3] = 255
            if not np.array_equal(atlas, before):
                diff = np.any(atlas != before, axis=2)
                rgb = before[..., :3].astype(np.int16)
                if kind == 'ground':
                    gray = before[..., 3] == 0
                else:
                    gray = (rgb[..., 0] == rgb[..., 1]) & (rgb[..., 1] == rgb[..., 2]) & (before[..., 3] == 255)
                require(not np.any(diff & ~gray), 'Fill changed a known texel: ' + image.name)
                gr.write_image(image, atlas)
            counts.setdefault('images', []).append({'slot': slot, 'image': image.name, 'size': list(image.size),
                                                    'sha256': hashlib.sha256(gr.read_image(image).tobytes()).hexdigest()})
        objects[obj.name] = counts
    totals = {key: sum(o[key] for o in objects.values()) for key in ('neutral_before', 'generated', 'unseen')}
    return {'asset_id': target_id, 'experiment': str(experiment),
            'generated_preserved_sha256': sha(generation / 'generated-preserved.png'),
            'views_sha256': sha(experiment / 'views.json'), 'totals': totals, 'objects': objects}


def render_actual(target_id, scene, gr, output):
    """Worker texels (no gray substitution) in the prepared cameras; residual unknown pixels counted."""
    from PIL import Image
    manifest = read(UNSEEN / target_id / 'views.json')
    target = Target(manifest['target'], scene, gr)
    lighting = manifest['lighting']
    bvh = bvh_for(target)
    sheet = np.zeros((manifest['layout']['height'], manifest['layout']['width'], 4), dtype=np.uint8)
    residual = 0
    for entry in manifest['views']:
        camera = Camera(entry['azimuth_degrees'], entry['elevation_degrees'], np.zeros((1, 3)), np.zeros((1, 3)), TILE)
        camera.matrix = np.array(entry['camera_matrix_world'])
        camera.rotation = camera.matrix[:3, :3]
        camera.toward = camera.rotation[:, 2]
        camera.scale = entry['ortho_scale']
        _, tri, coords = raster(camera, target.corners, SS)
        covered, color, unknown, _ = target.shade(camera, tri, coords, lighting, bvh)
        residual += int((unknown & covered).sum())
        c = entry['crop']
        tile = sheet[c['top']:c['top'] + TILE, c['left']:c['left'] + TILE]
        tile[covered, :3] = np.rint(color[covered]).astype(np.uint8)
        tile[covered, 3] = 255
    directory = output / 'renders' / target_id
    directory.mkdir(parents=True)
    Image.fromarray(sheet).save(directory / 'textured.png')
    return {'actual_sheet': str(directory / 'textured.png'), 'actual_sheet_sha256': sha(directory / 'textured.png'),
            'residual_unknown_pixels': residual}


def geometry_record(scene):
    digest = hashlib.sha256()
    for record in scene.meshes:
        obj = record['object']
        digest.update(obj.name.encode() + np.ascontiguousarray(record['corners']).tobytes() + record['slots'].tobytes())
        for layer in obj.data.uv_layers:
            data = np.empty(len(obj.data.loops) * 2, dtype=np.float32)
            layer.data.foreach_get('uv', data)
            digest.update(layer.name.encode() + data.tobytes())
    return digest.hexdigest()


def fill(worker_in, output, ids=None, include_unapproved=False, render=True):
    import bpy
    worker_in = Path(worker_in).resolve(strict=True)
    output = Path(output).resolve()
    require(not output.exists(), 'Output exists: ' + str(output))
    worker_sha = sha(worker_in)
    ids = ids or sorted(p.parent.parent.name for p in UNSEEN.glob('*/' + GENERATION + '/generation.json'))
    selected = [i for i in ids if include_unapproved or approved(i, UNSEEN / i)]
    skipped = sorted(set(ids) - set(selected))
    gr, scene = open_worker(worker_in)
    geometry_before = geometry_record(scene)
    output.mkdir(parents=True)
    assets, failures = [], {}
    for target_id in selected:
        try:
            assets.append(fill_target(target_id, scene, gr))
            print(json.dumps({'target': target_id, **assets[-1]['totals']}), flush=True)
        except Exception as error:
            failures[target_id] = f'{type(error).__name__}: {error}'
            print(json.dumps({'target': target_id, 'failed': failures[target_id]}), flush=True)
    import global_reproject
    require(geometry_record(full_scene(global_reproject)) == geometry_before, 'Fill changed geometry, UVs or slots')
    bpy.ops.wm.save_as_mainfile(filepath=str(output / 'worker.blend'))
    if render:
        scene = full_scene(gr)
        for row in assets:
            row.update(render_actual(row['asset_id'], scene, gr, output))
            print(json.dumps({'target': row['asset_id'], 'residual_unknown_pixels': row['residual_unknown_pixels']}), flush=True)
    write(output / 'combine.json', {
        'version': 1, 'kind': 'lincoln-unseen-fill', 'worker_in': str(worker_in), 'worker_in_sha256': worker_sha,
        'worker_out_sha256': sha(output / 'worker.blend'), 'geometry_sha256': geometry_before,
        'texture_approved_only': not include_unapproved, 'skipped_unapproved': skipped,
        'rule': ('Only receiver texels matching global_reproject.neutral_mask are written; best-facing first-hit '
                 'view of the displayed target state; generated-preserved.png sampled inside the silhouette '
                 'and the editable mask.'),
        'assets': assets, 'failures': failures})


# ---------------------------------------------------------------- review

def review(fill_output):
    """texture-review.json per target (pending inspection notes) and the texture review gallery."""
    sys.path.insert(0, str(EDITOR / 'refinement/blender'))
    from build_review_gallery import build
    fill_output = Path(fill_output).resolve()
    combine = read(fill_output / 'combine.json')
    items = []
    for row in combine['assets']:
        experiment = Path(row['experiment'])
        generation = experiment / GENERATION
        manifest = read(experiment / 'views.json')
        spec = manifest['target']
        review_path = experiment / 'texture-review.json'
        notes = read(review_path).get('notes', []) if review_path.exists() else []
        validation = fill_output / 'renders' / row['asset_id'] / 'validation.json'
        write(validation, {'fill': str(fill_output / 'combine.json'), 'fill_sha256': sha(fill_output / 'combine.json'),
                           'worker_out_sha256': combine['worker_out_sha256'], 'totals': row['totals'],
                           'residual_unknown_pixels': row['residual_unknown_pixels'],
                           'actual_sheet_sha256': row['actual_sheet_sha256'],
                           'generated_preserved_sha256': row['generated_preserved_sha256'],
                           'views_sha256': row['views_sha256'],
                           'camera_selection': manifest['camera_selection'],
                           'verification': str(fill_output / 'verification.json')})
        write(review_path, {'status': 'ready-for-user', 'fill': str(fill_output),
                            'actual_sheet_sha256': row['actual_sheet_sha256'],
                            'generated_preserved_sha256': row['generated_preserved_sha256'],
                            'views_sha256': row['views_sha256'], 'notes': notes})
        state = ('covered state' if not spec['patches'] else 'revealed state ' + ', '.join(spec['patches']))
        items.append({
            'id': row['asset_id'], 'name': spec.get('name', row['asset_id']), 'status': 'ready-for-user',
            'user_approval': 'pending',
            'notes': ['Texture approval pending. Only texels no source camera sees are generated; all other texels unchanged.',
                      f"{state}; receivers: {len(spec['receivers'])} objects; unknown texels before "
                      f"{row['totals']['neutral_before']:,}, generated {row['totals']['generated']:,}, "
                      f"still unseen {row['totals']['unseen']:,}; residual gray pixels in the eight views "
                      f"{row['residual_unknown_pixels']:,}.", *notes],
            'solid': str(experiment / 'solid.png'),
            'textured': row['actual_sheet'],
            'textured_label': 'Generated fill written into the worker atlases (actual texels) — approval candidate',
            'source_comparison': str(experiment / 'input.png'),
            'source_comparison_label': 'Published textures before the fill (gray = never seen by the source camera)',
            'source_comparison_secondary': str(generation / 'generated-preserved.png'),
            'source_comparison_secondary_label': 'Generated sheet with known pixels restored',
            'source_trace': str(generation / 'generated-raw.png'),
            'source_trace_label': 'Raw Sunburst output — reference only',
            'validation': str(validation), 'review': str(review_path)})
    manifest = UNSEEN / 'texture-candidates.json'
    write(manifest, {'map': 'Lincoln unseen-texel fill', 'review_kind': 'texture', 'items': items})
    build(manifest, UNSEEN / 'gallery', map_name='Lincoln unseen-texel fill', pending_only=True)
    print(json.dumps({'gallery': str(UNSEEN / 'gallery/index.html'), 'items': len(items)}))


def record_decisions(batch):
    """Record the user's gallery decisions (`<id>: <decision text> [review <prefix>]` lines).

    Only lines whose decision text starts with "approved" become texture approvals; each is
    bound to the displayed card's review revision and to the generated sheet and views hashes of
    its validation.json, which must still match the experiment files.
    """
    import re
    batch = Path(batch).resolve(strict=True)
    evidence = read(UNSEEN / 'gallery/evidence.json')
    cards = {item['id']: item for item in evidence['items']}
    records = read(DECISIONS)['decisions'] if DECISIONS.exists() else []
    rows = []
    for line in batch.read_text().splitlines():
        match = re.fullmatch(r'(\S+): (.+) \[review ([0-9a-f]{16})\]', line.strip())
        if not match:
            continue
        target_id, text, prefix = match.groups()
        card = cards.get(target_id)
        require(card and card['review_revision'].startswith(prefix),
                f'{target_id}: decision does not bind the displayed card revision {prefix}')
        if not text.startswith('approved'):
            rows.append({'asset_id': target_id, 'decision': 'not approved', 'text': text})
            continue
        validation = read(card['validation'])
        experiment = UNSEEN / target_id
        bound = {'generated_preserved_sha256': sha(experiment / GENERATION / 'generated-preserved.png'),
                 'views_sha256': sha(experiment / 'views.json'),
                 'actual_sheet_sha256': sha(card['textured'])}
        require(all(validation[k] == v for k, v in bound.items()),
                target_id + ': experiment files changed since the reviewed fill')
        if any(r['asset_id'] == target_id and r['review_revision'] == card['review_revision'] for r in records):
            continue
        records.append({'asset_id': target_id, 'scope': 'texture', 'decision': 'approved',
                        'review_revision': card['review_revision'],
                        'exact_user_text': f'{target_id}: {text} [review {prefix}]',
                        'source': {'path': str(batch), 'sha256': sha(batch)},
                        'evidence_sha256': bound})
        rows.append({'asset_id': target_id, 'decision': 'approved'})
    write(DECISIONS, {'version': 1, 'decisions': records})
    print(json.dumps(rows, indent=1))


def main():
    in_blender = '--' in sys.argv
    argv = sys.argv[sys.argv.index('--') + 1:] if in_blender else sys.argv[1:]
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    command = sub.add_parser('survey')
    command.add_argument('--worker', type=Path, required=True)
    command.add_argument('--output', type=Path, required=True)
    command = sub.add_parser('survey-visible')
    command.add_argument('--worker', type=Path, required=True)
    command.add_argument('--output', type=Path, required=True)
    command = sub.add_parser('prepare')
    command.add_argument('--worker', type=Path, required=True)
    command.add_argument('--target', action='append', required=True)
    command = sub.add_parser('generate')
    command.add_argument('ids', nargs='+')
    command.add_argument('--prompt-suffix')
    command = sub.add_parser('fill')
    command.add_argument('--worker-in', type=Path, required=True)
    command.add_argument('--output', type=Path, required=True)
    command.add_argument('--target', action='append')
    command.add_argument('--include-unapproved', action='store_true')
    command.add_argument('--no-render', action='store_true')
    command = sub.add_parser('review')
    command.add_argument('fill_output', type=Path)
    command = sub.add_parser('record-decisions')
    command.add_argument('batch', type=Path)
    args = parser.parse_args(argv)
    if args.command in ('survey', 'survey-visible', 'prepare', 'fill') and not in_blender:
        raise SystemExit(args.command + ' must run inside Blender')
    if args.command == 'survey':
        survey(args.worker, args.output)
    elif args.command == 'survey-visible':
        survey_visible(args.worker, args.output)
    elif args.command == 'prepare':
        prepare(args.worker, args.target)
    elif args.command == 'generate':
        generate(args.ids, args.prompt_suffix)
    elif args.command == 'fill':
        fill(args.worker_in, args.output, args.target, args.include_unapproved, not args.no_render)
    elif args.command == 'record-decisions':
        record_decisions(args.batch)
    else:
        review(args.fill_output)


if __name__ == '__main__':
    main()
