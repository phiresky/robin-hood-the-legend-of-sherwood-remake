"""Reconstruct a continuous native leaf envelope over inferred branch volumes."""
import argparse
import json
import math
from pathlib import Path
import shutil
import sys

import bpy
import numpy as np
from PIL import Image
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
                str(ROOT / 'level-editor/refinement/blender')]
from approved_texture_stage import geometry, appearance, require
from evidence_io import sha, write_json
from render_slots import acquire, release
from render_tree import render_workspace
from tree_geometry import SIN, COS, RAY, replace_mesh


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--flat-leaf-fragments', action='store_true',
                        help='Use small source-facing fragments instead of stretching the native chart over slopes')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    source, output = args.source.resolve(), args.output.resolve()
    require(not output.exists(), 'Use a fresh candidate destination')
    source_hash = sha(source / 'model.blend')
    cfg = json.loads((source / 'workspace.json').read_text())
    refinement = json.loads((source / 'inspection/refinement.json').read_text())
    packet = json.loads((source / 'inspection/source-packet/partition.json').read_text())
    source_image = source / 'inspection/source-packet/complete-source.png'
    alpha = np.asarray(Image.open(source_image).convert('RGBA'))[:, :, 3] > 127
    x0, y0, width, height = packet['native_bbox']
    require(alpha.shape == (height, width), 'Native source layout changed')
    centers = np.asarray(refinement['crown']['branch_clumps']['centers'])
    radii = np.asarray(refinement['crown']['branch_clumps']['radii']) * 1.13
    ray = np.array(RAY)
    ray /= np.linalg.norm(ray)

    def point(x, y):
        origin = np.array([x, -y*SIN, -y*COS])
        relative = origin - centers
        a = np.sum((ray/radii)**2, axis=1)
        b = 2*np.sum(relative*ray/radii**2, axis=1)
        c = np.sum((relative/radii)**2, axis=1)-1
        disc = b*b-4*a*c
        valid = disc >= 0
        high = (-b+np.sqrt(np.maximum(0, disc)))/(2*a)
        if np.any(valid):
            depth = float(high[valid].max())
        else:
            # Outside a fitted lobe the image remains authoritative. Extend the
            # closest lobe's tangent depth; do not discard peripheral leaves.
            projected = centers[:, 0:1] - np.array([[x]])
            cy = -centers[:, 1]*SIN-centers[:, 2]*COS
            distance = projected[:, 0]**2+(cy-y)**2
            depth = float(high[np.argmin(distance)])
        return origin + ray*(depth+2.)

    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source / 'model.blend'))
        crown, = [o for o in bpy.data.collections[cfg['collection_name']].all_objects
                  if o.type == 'MESH' and o.get('asset_group') == cfg['asset_id']
                  and o.get('projection_component') == 'crown']
        foreign = {o.name: (geometry(o), appearance(o)) for o in bpy.data.objects
                   if o.type == 'MESH' and o != crown}
        old = crown.data
        materials = list(old.materials)
        require(materials[0].get('foliage_observed') is True, 'Native material role changed')
        tex, = [n for n in materials[0].node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
        import hashlib
        require(tex.image.packed_file and hashlib.sha256(tex.image.packed_file.data).hexdigest() == sha(source_image),
                'Native complete-source image differs from packed material')
        vertices, faces, uvs, slots, known = [], [], [], [], []
        old_uv = old.uv_layers['Foliage UV']
        # Retain all genuinely inferred rear clusters and out-of-map continuation.
        for polygon in old.polygons:
            if polygon.material_index not in (4, 6):
                continue
            start = len(vertices)
            for loop in polygon.loop_indices:
                vertices.append(list(crown.matrix_world @ old.vertices[old.loops[loop].vertex_index].co))
                uvs.append(tuple(old_uv.data[loop].uv))
            faces.append(tuple(range(start, len(vertices))))
            slots.append(polygon.material_index)
            known.append(False)
        retained_faces = len(faces)
        for top in range(0, height, 4):
            for left in range(0, width, 4):
                right, bottom = min(width, left+4), min(height, top+4)
                if not alpha[top:bottom, left:right].any():
                    continue
                coordinates = [(left, top), (right, top), (right, bottom), (left, bottom)]
                points = [point(x0+x, y0+y) for x, y in coordinates]
                if args.flat_leaf_fragments:
                    middle = point(x0+(left+right)/2, y0+(top+bottom)/2)
                    depth = float(middle @ ray)
                    points = [np.array([x0+x, -(y0+y)*SIN, -(y0+y)*COS])+ray*depth
                              for x, y in coordinates]
                for backside in (False, True):
                    start = len(vertices)
                    vertices.extend([list(p-ray*.02) if backside else list(p) for p in points])
                    uvs.extend([(x/width, 1-y/height) for x, y in coordinates])
                    faces.extend([(start, start+1, start+2), (start, start+2, start+3)] if backside
                                 else [(start+2, start+1, start), (start+3, start+2, start)])
                    slots.extend([2 if backside else 0]*2)
                    known.extend([not backside]*2)
        result = replace_mesh(crown, vertices, faces, uvs, materials, slots, known)
        require(foreign == {o.name: (geometry(o), appearance(o)) for o in bpy.data.objects
                           if o.type == 'MESH' and o != crown}, 'Foreign or wood receiver changed')
        output.mkdir(parents=True)
        for relative in ['workspace.json', 'source-masks.json', 'modified/views.json',
                         'inspection/refinement.json', 'inspection/source-coverage/report.json']:
            target = output / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / relative, target)
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_as_mainfile(filepath=str(output / 'model.blend'), compress=True)
        write_json(output / 'inspection/envelope-preservation.json', dict(
            status='private geometry trial; review pending', source_worker=str(source),
            source_model_sha256=source_hash, model_sha256=sha(output / 'model.blend'),
            native_source_image=str(source_image), native_source_sha256=sha(source_image),
            original_native_rgba_image_reused_exactly=True, native_uv_mapping='exact source x/y orthographic projection',
            flat_leaf_fragments=args.flat_leaf_fragments,
            retained_inferred_faces=retained_faces, mesh=result,
            non_crown_geometry_and_appearance_unchanged=True,
            method='Smooth native-source leaf envelope over branch-scale ellipsoids; original hidden leaf clusters and off-map continuation retained',
            approval='new geometry, no inherited approval; no API generation'))
        render_workspace(output, 384, release_slot=False, transparent_bounces=256)
        require(sha(source / 'model.blend') == source_hash, 'Original worker changed')
    finally:
        release()


if __name__ == '__main__':
    main()
