"""Round exposed rock edges while preserving surveyed relief and ownership."""
import argparse
import json
import sys
import uuid
import math
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector, noise

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha, write_json
from refinement_workspace import prepare, validate, modified
from render_slots import acquire, release
from audit_candidates import audit
from render_tree import render_workspace
from tree_geometry import replace_mesh, SIN, COS

ASSETS = ('northwest-rock-outcrop', 'west-rock-outcrop', 'southwest-rock-outcrop', 'northeast-oak-root-bank')


def sculpt(obj):
    """A broad edge treatment and restrained surface relief, not random boulders."""
    level = json.loads((OUT / 'baseline/Croisement02.rhp.json').read_text())
    points = level['sight_obstacles'][int(obj['source_node'].split('-')[-1])]['points']
    count = len(points)
    span_x=max(p['x'] for p in points)-min(p['x'] for p in points)
    extension=max(36.,min(100.,span_x*.65)) if min(p['x'] for p in points)<=.5 else 0.
    vertices = [(p['x']-extension if p['x']<=.5 else p['x'], -p['y'] / SIN, p[z] / COS)
                for z in ('z_bottom', 'z_top') for p in points]
    faces = [tuple(reversed(range(count))), tuple(range(count, count * 2))]
    faces += [(i, (i + 1) % count, (i + 1) % count + count, i + count) for i in range(count)]
    # Reconstruct a shared-vertex prism: imported material faces have small
    # per-face offsets and cannot be welded without changing their positions.
    replace_mesh(obj, vertices, faces, materials=list(obj.data.materials))
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.001)
    # Imported occlusion prisms omit their bottom cap. Close that unseen base
    # before beveling so the standalone rock remains a watertight asset.
    bmesh.ops.holes_fill(bm, edges=[e for e in bm.edges if e.is_boundary], sides=0)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(obj.data)
    bm.free()
    world = [obj.matrix_world @ v.co for v in obj.data.vertices]
    spans = [max(p[i] for p in world) - min(p[i] for p in world) for i in range(3)]
    radius = max(1., min(12., min(spans) * .20))
    bpy.ops.object.select_all(action='DESELECT')
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    modifier = obj.modifiers.new('Weathered exposed corners', 'BEVEL')
    modifier.width = radius
    modifier.segments = 4
    modifier.affect = 'EDGES'
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    obj.data.remesh_voxel_size = min(1.25, max(.5, radius * .15))
    obj.data.use_remesh_preserve_volume = True
    bpy.ops.object.voxel_remesh()
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.smooth_vert(bm, verts=list(bm.verts), factor=.15,
                         use_axis_x=True, use_axis_y=True, use_axis_z=True)
    bm.normal_update()
    inverse = obj.matrix_world.inverted()
    normal_matrix = obj.matrix_world.to_3x3().inverted().transposed()
    west = min(p.x for p in world)
    middle_y = (min(p.y for p in world) + max(p.y for p in world)) * .5
    middle_z = (min(p.z for p in world) + max(p.z for p in world)) * .5
    half_y = spans[1] * .5
    half_z = spans[2] * .5
    for vertex in bm.verts:
        p = obj.matrix_world @ vertex.co
        normal = (normal_matrix @ vertex.normal).normalized()
        wave = noise.noise_vector(p*.06+Vector((3.1,7.3,11.7))).x
        envelope = min(1., max(0., p.z) / 8.)
        p += normal * (wave * min(.8, radius * .12) * envelope)
        if extension and p.x < 0:
            # A squared prism is only a visibility proxy. Complete its unseen
            # return as a rounded rock body, continuously joined at the map
            # edge. This cube-to-ellipsoid mapping preserves all in-map points.
            t = min(1., p.x / west)
            y = max(-1., min(1., (p.y-middle_y)/half_y))
            z = max(-1., min(1., (p.z-middle_z)/half_z))
            p.x *= math.sqrt(max(.25, 1.-.38*y*y-.32*z*z))
            p.y = middle_y + (p.y-middle_y)*math.sqrt(1.-.6*t*t)
            p.z = middle_z + (p.z-middle_z)*math.sqrt(1.-.6*t*t)
            # Restrained asymmetry on the inferred cap avoids a manufactured
            # ellipsoid; this fades to zero exactly at the source boundary.
            p += normal * noise.noise_vector(p*.027+Vector((9,3,5))).x * (3.*t*t)
        p.z = max(0., p.z)
        vertex.co = inverse @ p
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(obj.data)
    bm.free()
    # Rebuild via the standard helper for topology checks and fresh source UVs.
    result = replace_mesh(obj, [tuple(obj.matrix_world @ v.co) for v in obj.data.vertices],
                          [tuple(p.vertices) for p in obj.data.polygons], materials=list(obj.data.materials))
    if result['nonmanifold_edges']:
        raise ValueError('Rock must remain a closed surface: ' + str(result))
    result.update(source_node=obj['source_node'], edge_radius=radius,
                  inferred_west_extension=extension,
                  inferred_return_shape='rounded asymmetric cap; source-side points untouched',
                  method='Surveyed relief with rounded exposed corners and low-amplitude inferred weathering')
    return result


def revise(slug,redo=False):
    asset = 'croisement02-' + slug
    old = OUT / 'scenery-round-1/assets' / asset
    worker = OUT / 'scenery-round-2/assets' / asset
    receipt = worker / 'inspection/relief-revision.json'
    latest = {r['asset_id']: r for r in json.loads((OUT / 'user-feedback.json').read_text())['records']}
    if latest.get(asset, {}).get('decision') == 'approved':
        raise ValueError('Approved geometry is frozen')
    directory = OUT / 'ownership-revision'
    catalog = json.loads((directory / 'catalog.json').read_text())
    owners = {f"building-{p['obstacle']:03}": g for g in catalog['groups'] for p in g['parts']}
    acquire()
    try:
        if receipt.exists() and redo:
            receipt.rename(receipt.with_name('relief-revision-archive-'+uuid.uuid4().hex[:8]+'.json'))
        if not (worker / 'workspace.json').exists():
            bpy.ops.wm.open_mainfile(filepath=str(OUT / 'forest-v4-input.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            for obj in bpy.data.collections['Croisement02 Working'].all_objects:
                if obj.type == 'MESH' and obj.get('source_node') in owners:
                    owner = owners[obj['source_node']]
                    obj['asset_group'] = owner['id']
                    obj['asset_name'] = owner['name']
            prepare(worker, asset_id=asset, scene_name='Croisement02 Refinement',
                collection_name='Croisement02 Working',
                source_path=OUT / 'animation-references/composite-frame-0.png',
                grouping_manifest=directory / 'catalog.json',
                inventory_path=OUT / 'forest-v4-inventory/inventory.json',
                review_path=directory / 'grouping-review.json',
                source_mask_manifest=old / 'source-masks.json', width=384, height=384,
                framing_padding=1.3,
                lighting=dict(toward_sun=[-.45, -.55, .70], ambient=.22, diffuse=.78, shadow_epsilon=.05))
        if not receipt.exists():
            bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            validate(worker)
            objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                       if o.type == 'MESH' and o.get('asset_group') == asset]
            parts = [sculpt(obj) for obj in objects]
            modified(worker)
            (worker / 'inspection').mkdir(exist_ok=True)
            report = dict(asset_id=asset, model_sha256=sha(worker / 'model.blend'), parts=parts,
                status='geometry candidate; visual review pending',
                limitations=['Hidden rock faces remain source-only gray pending texture completion.',
                             'Native footprints and relief determine placement. Rounded edges and small surface weathering are inferred.',
                             'Rock bodies meeting the western image edge continue beyond it with an inferred closed return; hidden shape is not source evidence.',
                             'Full-scene terrain contact and foreground coverage require integrated review.'])
            write_json(worker / 'inspection/refinement.json', report)
            audit(worker)
            write_json(receipt, dict(model_sha256=report['model_sha256'],
                previous_model_sha256=sha(old / 'model.blend'), catalog_sha256=sha(directory / 'catalog.json')))
        elif json.loads(receipt.read_text())['model_sha256'] != sha(worker / 'model.blend'):
            raise ValueError('Revised relief changed')
        render_workspace(worker, 384, release_slot=False)
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--assets', nargs='+', choices=ASSETS, default=list(ASSETS))
    parser.add_argument('--redo',action='store_true')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else [])
    for slug in args.assets:
        revise(slug,args.redo)
