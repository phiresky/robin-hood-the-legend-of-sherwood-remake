"""Add reviewed revealed/animated state geometry to an approved Lincoln model.

blender --background <approved model.blend> --threads 2 --python-exit-code 1 \
  --python level-editor/blender/lincoln/revealed_state_geometry.py -- <spec.json> <out-dir>

The approved covered geometry is never edited. Every state change is expressed as
whole-object visibility, which is what the editor's patch display supports:

- ``hide``: existing owned objects get ``reveal_hide_when_applied`` patch IDs.
- ``show``: existing owned objects get ``reveal_show_when_applied`` (hidden while
  covered, e.g. an open door leaf that has no covered-state artwork).
- ``variants``: a copy of one owned object, reshaped by explicit operations, is
  shown when its patches are applied; the original is hidden for those patches.
- ``additions``: new owned surfaces (floors, walls, sprite silhouettes) shown only
  for their patches.

Coordinates are native Lincoln units: pixel_x = x, pixel_y = y - z (35 degree
source camera); world = (x, -y/sin35, z/cos35). Variant/addition objects keep the
owning source_node and asset_group and carry a distinct projection_component.
Every original object's geometry, UVs, materials and transform are fingerprinted
before and after; any change fails the run. Output: <out-dir>/state-model.blend
and <out-dir>/state-geometry.json. Visibility defaults to the covered state.
"""
import hashlib
import json
import math
from pathlib import Path
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector

S, C = math.sin(math.radians(35)), math.cos(math.radians(35))
TAG = 'lincoln-revealed-states-v1'
STATE_SHOWN = {}  # (source_node, patches) -> variant built so far in this run


def world(p):
    x, y, z = p
    return Vector((x, -y / S, z / C))


def world_normal(n):
    x, y, z = n
    return Vector((x, -y * S, z * C)).normalized()


def fingerprint(obj):
    mesh = obj.data
    value = {'v': [[round(c, 6) for c in v.co] for v in mesh.vertices],
             'f': [list(p.vertices) for p in mesh.polygons],
             'mi': [p.material_index for p in mesh.polygons],
             'm': [[round(c, 6) for c in row] for row in obj.matrix_world],
             'mat': [m.name if m else None for m in mesh.materials],
             'uv': {u.name: [[round(c, 6) for c in d.uv] for d in u.data] for u in mesh.uv_layers},
             'hide_render': obj.hide_render}
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def fill_cut(bm, edges):
    boundary = [e for e in edges if e.is_valid and e.is_boundary]
    if boundary:
        bmesh.ops.holes_fill(bm, edges=boundary, sides=0)


def clip(obj, point, normal):
    """Remove the half-space on the positive side of the world plane and cap it."""
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    inv = obj.matrix_world.inverted()
    local_point = inv @ point
    local_normal = (inv.to_3x3().inverted().transposed() @ normal).normalized()
    geom = list(bm.verts) + list(bm.edges) + list(bm.faces)
    result = bmesh.ops.bisect_plane(bm, geom=geom, plane_co=local_point, plane_no=local_normal,
                                    clear_outer=True, dist=1e-4)
    cut_edges = [g for g in result['geom_cut'] if isinstance(g, bmesh.types.BMEdge)]
    fill_cut(bm, cut_edges)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(obj.data)
    bm.free()
    obj.data.update()


def prism_mesh(name, polygon, z0, z1):
    verts = [world((x, y, z0)) for x, y in polygon] + [world((x, y, z1)) for x, y in polygon]
    n = len(polygon)
    faces = [tuple(range(n))[::-1], tuple(range(n, 2 * n))]
    faces += [(i, (i + 1) % n, (i + 1) % n + n, i + n) for i in range(n)]
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata([tuple(v) for v in verts], [], faces)
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(mesh)
    bm.free()
    return mesh


def sprite_mesh(name, image, bbox, plane_y, depth):
    """Opaque sprite pixels as a card on the native plane y=plane_y, extruded depth units north.

    A constant-native-y plane projects exactly onto the sprite rectangle in the
    source view (pixel (px, py) -> native (px, plane_y, plane_y - py)); the front
    is the exact silhouette and the concealed depth is an explicit inference.
    """
    import numpy as np
    from PIL import Image
    path = Path(__file__).resolve().parents[2] / 'work/needs-review/lincoln-refinement/source-states' / image
    alpha = np.asarray(Image.open(path).convert('RGBA'))[:, :, 3] > 127
    left, top, width, height = bbox
    if alpha.shape != (height, width) or not alpha.any():
        raise ValueError('Sprite silhouette does not match its bbox: ' + image)
    index, verts, faces = {}, [], []

    def vertex(x, y, d):
        key = (x, y, d)
        if key not in index:
            index[key] = len(verts)
            verts.append(tuple(world((left + x, plane_y + d, plane_y + d - (top + y)))))
        return index[key]
    occupied = {(x, y) for y, x in zip(*np.nonzero(alpha))}
    for x, y in sorted(occupied):
        faces.append(tuple(vertex(a, b, 0) for a, b in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)]))
        faces.append(tuple(vertex(a, b, depth) for a, b in [(x, y + 1), (x + 1, y + 1), (x + 1, y), (x, y)]))
        for other, (a, b) in [((x - 1, y), ((x, y), (x, y + 1))), ((x + 1, y), ((x + 1, y + 1), (x + 1, y))),
                              ((x, y - 1), ((x + 1, y), (x, y))), ((x, y + 1), ((x, y + 1), (x + 1, y + 1)))]:
            if other not in occupied:
                faces.append((vertex(*a, 0), vertex(*b, 0), vertex(*b, depth), vertex(*a, depth)))
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], faces)
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=1e-6)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(mesh)
    bm.free()
    return mesh


def _clip_area(subject, clipper):
    """Area of a convex polygon intersection (Sutherland-Hodgman, 2D)."""
    def side(p, a, b):
        return (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])

    def cross(p, q, a, b):
        x1, y1 = p; x2, y2 = q; x3, y3 = a; x4, y4 = b
        den = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4)
        if abs(den) < 1e-12:
            return q
        t = ((x1 - x3) * (y3 - y4) - (y1 - y3) * (x3 - x4)) / den
        return (x1 + t * (x2 - x1), y1 + t * (y2 - y1))

    def signed(poly):
        return sum(poly[i][0] * poly[i - 1][1] - poly[i - 1][0] * poly[i][1] for i in range(len(poly))) * -0.5
    if signed(clipper) < 0:
        clipper = clipper[::-1]
    out = subject
    for i in range(len(clipper)):
        a, b = clipper[i], clipper[(i + 1) % len(clipper)]
        points, out = out, []
        for j in range(len(points)):
            p, q = points[j - 1], points[j]
            if side(q, a, b) >= 0:
                if side(p, a, b) < 0:
                    out.append(cross(p, q, a, b))
                out.append(q)
            elif side(p, a, b) >= 0:
                out.append(cross(p, q, a, b))
        if not out:
            return 0.0
    return abs(signed(out)) if len(out) > 2 else 0.0


def triangles(obj):
    mesh = obj.data
    mesh.calc_loop_triangles()
    rows = []
    for tri in mesh.loop_triangles:
        points = [obj.matrix_world @ mesh.vertices[v].co for v in tri.vertices]
        normal = (points[1] - points[0]).cross(points[2] - points[0])
        if normal.length < 1e-9:
            continue
        rows.append((tri.polygon_index, points, normal.normalized()))
    return rows


def coplanar_pairs(a, b, tolerance=0.3, minimum=1.0, visible_only=False):
    """Same-facing triangle pairs of two meshes within `tolerance` and overlapping > minimum area.

    visible_only skips downward-facing pairs (undersides; no editor view looks up at them).
    """
    result = []
    tb = triangles(b)
    boxes = [(min(p[i] for p in pts) - tolerance, max(p[i] for p in pts) + tolerance) for _, pts, _ in tb
             for i in range(3)]
    for ia, pa, na in triangles(a):
        lo = [min(p[i] for p in pa) for i in range(3)]
        hi = [max(p[i] for p in pa) for i in range(3)]
        u = (pa[1] - pa[0]).normalized()
        v = na.cross(u)
        flat_a = [((p - pa[0]).dot(u), (p - pa[0]).dot(v)) for p in pa]
        for k, (ib, pb, nb) in enumerate(tb):
            if any(hi[i] < boxes[3 * k + i][0] or lo[i] > boxes[3 * k + i][1] for i in range(3)):
                continue
            if na.dot(nb) < 0.999 or abs((pb[0] - pa[0]).dot(na)) > tolerance:
                continue
            if visible_only and na.z < -0.9:
                continue
            area = _clip_area(flat_a, [((p - pa[0]).dot(u), (p - pa[0]).dot(v)) for p in pb])
            if area > minimum:
                result.append((ia, ib, area))
    return result


def coplanar_faces(obj, other, tolerance=0.3, visible_only=False):
    """Faces of obj in a coplanar overlap with other, measured from both sides (the plane
    distance and clip use one triangle's frame, so near-parallel pairs are not symmetric)."""
    return ({ia for ia, _, _ in coplanar_pairs(obj, other, tolerance, visible_only=visible_only)} |
            {ib for _, ib, _ in coplanar_pairs(other, obj, tolerance, visible_only=visible_only)})


def circle(center, radius, segments=32):
    """Native plan outline of a world-space circle; radius in world (native x) units."""
    cx, cy = center
    return [(cx + radius * math.cos(2 * math.pi * i / segments),
             cy + radius * S * math.sin(2 * math.pi * i / segments)) for i in range(segments)]


def volume(mesh):
    bm = bmesh.new()
    bm.from_mesh(mesh)
    value = bm.calc_volume(signed=True)
    bm.free()
    return value


def subtract(obj, cutter_mesh):
    if volume(obj.data) < 0:
        # Some approved solids are wound inside-out; the copy is re-oriented so the
        # boolean treats it as a solid. The approved original is not touched.
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        bmesh.ops.reverse_faces(bm, faces=list(bm.faces))
        bm.to_mesh(obj.data)
        bm.free()
        obj['state_reoriented'] = True
    before = volume(obj.data)
    cutter = bpy.data.objects.new(obj.name + ' cutter', cutter_mesh)
    bpy.context.scene.collection.objects.link(cutter)
    modifier = obj.modifiers.new('state cut', 'BOOLEAN')
    modifier.operation = 'DIFFERENCE'
    # EXACT collapses these large-coordinate solids in Blender 5.2; the manifold
    # solver is exact for the closed inputs used here and is volume-checked below.
    modifier.solver = 'MANIFOLD'
    modifier.object = cutter
    depsgraph = bpy.context.evaluated_depsgraph_get()
    depsgraph.update()
    evaluated = obj.evaluated_get(depsgraph)
    mesh = bpy.data.meshes.new_from_object(evaluated, preserve_all_data_layers=True, depsgraph=depsgraph)
    obj.modifiers.remove(modifier)
    old = obj.data
    obj.data = mesh
    mesh.name = old.name
    bpy.data.objects.remove(cutter, do_unlink=True)
    bpy.data.meshes.remove(cutter_mesh)
    if old.users == 0:
        bpy.data.meshes.remove(old)
    after = volume(obj.data)
    # A cutter may legitimately miss (the approved gatehouse cavity cylinder removes
    # nothing); approved state models were built with this non-strict rule.
    if not (0 < after <= before + 1e-3 * abs(before)) or not obj.data.polygons:
        raise ValueError(f'Boolean subtraction produced an invalid solid for {obj.name}: {before} -> {after}')


def apply_op(obj, op):
    kind = op['op']
    if kind == 'clip_above':
        clip(obj, world((0, 0, op['z'])), Vector((0, 0, 1)))
    elif kind == 'clip_below':
        clip(obj, world((0, 0, op['z'])), Vector((0, 0, -1)))
    elif kind == 'clip_halfspace':
        clip(obj, world(op['point']), world_normal(op['normal']))
    elif kind == 'subtract_prism':
        if topology(obj)['nonmanifold_edges']:
            # Open approved sheets cannot be boolean-subtracted; cut them exactly
            # along the prism and drop the enclosed faces instead.
            apply_op(obj, {**op, 'op': 'delete_faces_in_prism'})
            obj['state_open_cut'] = True
        else:
            subtract(obj, prism_mesh(obj.name + ' cutter', op['polygon'], *op['z']))
    elif kind == 'subtract_cylinder':
        subtract(obj, prism_mesh(obj.name + ' cutter', circle(op['center'], op['radius'],
                                                               op.get('segments', 48)), *op['z']))
    elif kind == 'delete_faces_in_prism':
        # For open surfaces (terrain) the boolean solvers need closed inputs; remove
        # every face whose centroid lies in the plan polygon and z range instead.
        from mathutils.geometry import intersect_point_tri_2d
        polygon = [Vector(p) for p in op['polygon']]
        z0, z1 = op['z']
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        # Split the surface along the prism's vertical sides and floor first, so the
        # hole follows the prism exactly instead of the terrain's triangle size.
        inverse = obj.matrix_world.inverted()
        normal_matrix = inverse.to_3x3().inverted().transposed()
        planes = [(world((a[0], a[1], 0)), world_normal((-(b[1] - a[1]), b[0] - a[0], 0)))
                  for a, b in zip(op['polygon'], op['polygon'][1:] + op['polygon'][:1])]
        planes.append((world((0, 0, z0)), Vector((0, 0, 1))))
        planes.append((world((0, 0, z1)), Vector((0, 0, 1))))
        for point, normal in planes:
            geom = list(bm.verts) + list(bm.edges) + list(bm.faces)
            bmesh.ops.bisect_plane(bm, geom=geom, plane_co=inverse @ point,
                                   plane_no=(normal_matrix @ normal).normalized(), dist=1e-4)
        doomed = []
        for face in bm.faces:
            c = obj.matrix_world @ face.calc_center_median()
            x, y, z = c.x, -c.y * S, c.z * C
            inside = False
            j = len(polygon) - 1
            for i in range(len(polygon)):
                a, b = polygon[i], polygon[j]
                if (a.y > y) != (b.y > y) and x < (b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x:
                    inside = not inside
                j = i
            if inside and z0 <= z <= z1:
                doomed.append(face)
        if not doomed:
            raise ValueError('delete_faces_in_prism removed nothing from ' + obj.name)
        bmesh.ops.delete(bm, geom=doomed, context='FACES')
        bm.to_mesh(obj.data)
        bm.free()
        obj.data.update()
    elif kind == 'inset_coplanar_with':
        # Resolve same-facing coplanar overlaps with what the state shows of other owned nodes
        # (an earlier variant for the same patches, else the approved mesh): overlapping faces of
        # this copy move `distance` world units inward, so the hidden side goes behind (the
        # round-8 rule). Moving shared vertices can bring a neighbour face into a new overlap,
        # so passes repeat until none is left. Downward undersides stay flush (never visible).
        references = []
        for other in op['others']:
            shown_variant = op.get('_references', {}).get(other)
            reference = [shown_variant] if shown_variant is not None else [
                o for o in bpy.data.objects if o.type == 'MESH' and o.get('source_node') == other
                and o.get('asset_group') == obj.get('asset_group') and not o.get('state_recipe')
                and not o.get('state_context_recipe')]
            if len(reference) != 1:
                raise ValueError('inset_coplanar_with needs one shown mesh for ' + other)
            references.append(reference[0])
        mesh = obj.data
        inverse = obj.matrix_world.inverted()
        moved = []
        for attempt in range(6):
            flagged = set()
            for reference in references:
                flagged |= coplanar_faces(obj, reference, op.get('tolerance', 0.3), visible_only=True)
            if not flagged:
                break
            if attempt == 5:
                raise ValueError(f'inset_coplanar_with could not separate {obj.name}: faces {sorted(flagged)}')
            moves = {}
            for index in flagged:
                polygon = mesh.polygons[index]
                normal = (obj.matrix_world.to_3x3() @ polygon.normal).normalized()
                for v in polygon.vertices:
                    moves.setdefault(v, Vector()).__iadd__(normal)
            for v, direction in moves.items():
                position = obj.matrix_world @ mesh.vertices[v].co
                mesh.vertices[v].co = inverse @ (position - direction.normalized() * op.get('distance', 2.0))
            mesh.update()
            moved.append(sorted(flagged))
        if not moved:
            raise ValueError(f"inset_coplanar_with found no overlap between {obj.name} and {op['others']}")
        obj['state_inset_faces'] = json.dumps({'others': op['others'], 'passes': moved})
    elif kind == 'rotate_hinge':
        a, b = world(op['p0']), world(op['p1'])
        axis = (b - a).normalized()
        rotation = (Matrix.Translation(a) @ Matrix.Rotation(math.radians(op['angle_deg']), 4, axis)
                    @ Matrix.Translation(-a))
        obj.data.transform(obj.matrix_world.inverted() @ rotation @ obj.matrix_world)
        obj.data.update()
    else:
        raise ValueError('Unknown state geometry operation: ' + kind)


def topology(obj):
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    report = {'faces': len(bm.faces), 'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
              'degenerate_faces': sum(f.calc_area() < 1e-8 for f in bm.faces)}
    bm.free()
    return report


def owned(asset, node):
    """Owned objects of a node; asset None selects a context (other-asset) object."""
    matches = [o for o in bpy.data.objects if o.type == 'MESH' and o.get('source_node') == node
               and not o.get('state_recipe') and not o.get('state_context_recipe')
               and (o.get('asset_group') == asset if asset else True)]
    return matches


def one(asset, node, name=None):
    matches = owned(asset, node)
    if name:
        matches = [o for o in matches if o.name == name]
    if len(matches) != 1:
        raise ValueError(f'Expected one owned object for {node} ({name}); found {[o.name for o in matches]}')
    return matches[0]


def mark(obj, key, patches):
    if not patches or any(not isinstance(p, str) or not p.startswith('patch-') for p in patches):
        raise ValueError('Invalid patch list for ' + obj.name)
    existing = list(obj.get(key, []))
    obj[key] = sorted(set(existing) | set(patches))


def world_digest(obj):
    """Order-independent digest of world-space triangles (rounded to 1e-3 units)."""
    mesh = obj.data
    mesh.calc_loop_triangles()
    tris = []
    for tri in mesh.loop_triangles:
        corners = sorted(tuple(round(c, 3) for c in obj.matrix_world @ mesh.vertices[v].co) for v in tri.vertices)
        tris.append(corners)
    tris.sort()
    return hashlib.sha256(json.dumps(tris).encode()).hexdigest()


def main():
    spec_path, out_dir = sys.argv[sys.argv.index('--') + 1:]
    spec = json.loads(Path(spec_path).read_text())
    out = Path(out_dir).resolve()
    out.mkdir(parents=True, exist_ok=True)
    report = build(spec, Path(spec_path))
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(out / 'state-model.blend'))
    report['state_model_sha256'] = hashlib.sha256((out / 'state-model.blend').read_bytes()).hexdigest()
    (out / 'state-geometry.json').write_text(json.dumps(report, indent=2) + '\n')
    print('STATE-GEOMETRY', json.dumps({v['object']: v['topology'] for v in report['variants'] + report['additions']}))


def build(spec, spec_path, include_context=True):
    """Apply one asset's state spec to the open blend; returns the report (no save).

    include_context=False (for a staged whole-map worker, where every neighbour's own
    state geometry is built from its own spec) skips review-only context copies.
    """
    asset = spec['asset_id']
    source_blend = bpy.data.filepath
    originals = [o for o in bpy.data.objects if o.type == 'MESH' and not o.get('state_recipe')]
    before = {o.name: fingerprint(o) for o in originals}
    for o in originals:
        if o.get('asset_group') == asset and o.hide_render:
            raise ValueError('Approved model has a hidden owned object; state visibility would be ambiguous: ' + o.name)
    report = {'version': 1, 'recipe': TAG, 'asset_id': asset, 'spec': str(Path(spec_path).resolve()),
              'spec_sha256': hashlib.sha256(Path(spec_path).read_bytes()).hexdigest(),
              'source_blend': source_blend,
              'source_blend_sha256': hashlib.sha256(Path(source_blend).read_bytes()).hexdigest(),
              'states': spec['states'], 'hide': {}, 'show': {}, 'variants': [], 'additions': []}
    props = {}
    for node, patches in spec.get('hide', {}).items():
        for obj in owned(asset, node):
            props.setdefault(obj.name, {}).setdefault('reveal_hide_when_applied', set()).update(patches)
            report['hide'].setdefault(node, []).append(obj.name)
        if not owned(asset, node):
            raise ValueError('Hide target is not owned: ' + node)
    for node, patches in spec.get('show', {}).items():
        obj = one(asset, node)
        props.setdefault(obj.name, {}).setdefault('reveal_show_when_applied', set()).update(patches)
        report['show'][node] = obj.name
    created = []
    for variant in spec.get('variants', []):
        base = one(asset, variant['source_node'], variant.get('object'))
        obj = base.copy()
        obj.data = base.data.copy()
        obj.name = f"{base.name} :: {variant['component']}"
        obj.data.name = obj.name
        for collection in base.users_collection:
            collection.objects.link(obj)
        for op in variant['ops']:
            if op['op'] == 'inset_coplanar_with':
                # References are what this state shows: an earlier variant of that node for the
                # same patches, else the approved mesh. Cut faces of neighbours count too.
                op = {**op, '_references': {other: STATE_SHOWN.get((other, tuple(variant['patches'])))
                                            for other in op['others']}}
            apply_op(obj, op)
        STATE_SHOWN[(variant['source_node'], tuple(variant['patches']))] = obj
        obj['projection_component'] = variant['component']
        obj['state_recipe'] = TAG
        obj['state_variant_of'] = base.name
        obj['state_note'] = variant.get('note', '')
        obj.hide_render = True
        created.append((obj, variant['patches'], variant.get('hide_patches', [])))
        if variant.get('hide_original', True):
            props.setdefault(base.name, {}).setdefault('reveal_hide_when_applied', set()).update(variant['patches'])
        report['variants'].append({'object': obj.name, 'variant_of': base.name, 'source_node': variant['source_node'],
                                   'component': variant['component'], 'patches': variant['patches'],
                                   'hide_patches': variant.get('hide_patches', []),
                                   'ops': variant['ops'], 'note': variant.get('note', ''), 'topology': topology(obj)})
    for variant in spec.get('context_variants', []) if include_context else []:
        # Review-only: a neighbour's state geometry so this asset's packet has the
        # right occluders. Never exported; that neighbour reviews its own state.
        base = one(None, variant['source_node'], variant.get('object'))
        if base.get('asset_group') == asset:
            raise ValueError('context_variants must target other assets')
        obj = base.copy()
        obj.data = base.data.copy()
        obj.name = f"{base.name} :: context {variant['component']}"
        for collection in base.users_collection:
            collection.objects.link(obj)
        for op in variant['ops']:
            apply_op(obj, op)
        obj['projection_component'] = variant['component']
        obj['state_context_recipe'] = TAG
        obj.hide_render = True
        mark(obj, 'reveal_show_when_applied', variant['patches'])
        base['state_context_hide_when_applied'] = sorted(set(variant['patches']))
        report.setdefault('context_variants', []).append({'object': obj.name, 'variant_of': base.name,
                                                         'patches': variant['patches'], 'ops': variant['ops']})
    for addition in spec.get('additions', []):
        template = one(asset, addition['source_node'], addition.get('object'))
        if addition['kind'] == 'prism':
            mesh = prism_mesh(addition['component'], addition['polygon'], *addition['z'])
        elif addition['kind'] == 'cylinder':
            mesh = prism_mesh(addition['component'], circle(addition['center'], addition['radius'],
                                                            addition.get('segments', 48)), *addition['z'])
        elif addition['kind'] == 'sprite':
            mesh = sprite_mesh(addition['component'], addition['image'], addition['bbox'],
                               addition['plane_y'], addition.get('depth', 2.0))
        else:
            raise ValueError('Unknown addition kind')
        obj = bpy.data.objects.new(f"{template.name} :: {addition['component']}", mesh)
        for collection in template.users_collection:
            collection.objects.link(obj)
        for key in template.keys():
            if key in ('source_node', 'asset_group', 'asset_name', 'source_obstacle', 'part_name'):
                obj[key] = template[key]
        for material in template.data.materials:
            obj.data.materials.append(material)
        for op in addition.get('ops', []):
            apply_op(obj, op)
        obj['projection_component'] = addition['component']
        obj['state_recipe'] = TAG
        obj['state_note'] = addition.get('note', '')
        obj.hide_render = True
        created.append((obj, addition['patches'], addition.get('hide_patches', [])))
        report['additions'].append({'object': obj.name, 'source_node': addition['source_node'],
                                    'component': addition['component'], 'patches': addition['patches'],
                                    'hide_patches': addition.get('hide_patches', []),
                                    'definition': {k: v for k, v in addition.items() if k not in ('note',)},
                                    'note': addition.get('note', ''), 'topology': topology(obj)})
    for obj, patches, hide_patches in created:
        mark(obj, 'reveal_show_when_applied', patches)
        if hide_patches:
            mark(obj, 'reveal_hide_when_applied', hide_patches)
    # Visibility properties on approved objects are metadata only; geometry is untouched.
    for name, values in props.items():
        obj = bpy.data.objects[name]
        for key, patches in values.items():
            mark(obj, key, sorted(patches))
    after = {o.name: fingerprint(o) for o in originals}
    drift = sorted(n for n in before if before[n] != after[n])
    if drift:
        raise ValueError('Approved geometry changed: ' + ', '.join(drift))
    report['original_objects_unchanged'] = len(before)
    report['state_properties'] = {
        o.name: {k: list(o[k]) for k in ('reveal_hide_when_applied', 'reveal_show_when_applied') if k in o}
        for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == asset
        and ('reveal_hide_when_applied' in o or 'reveal_show_when_applied' in o)}
    report['world_digests'] = {row['object']: world_digest(bpy.data.objects[row['object']])
                               for row in report['variants'] + report['additions']}
    return report


if __name__ == '__main__':
    main()
