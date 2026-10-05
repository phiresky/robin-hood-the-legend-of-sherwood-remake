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
    parser.add_argument('--minimum-depth-width', type=float, default=0.,
                        help='Optional new inferred depth target, using positive native-ray scaling')
    parser.add_argument('--fragment-jitter', type=float, default=0.,
                        help='Bounded deterministic source-ray depth variation per leaf fragment')
    parser.add_argument('--soft-branch-envelope', action='store_true',
                        help='Private smooth branch falloff trial without ellipsoid tangent lips')
    parser.add_argument('--west-edge-completion', action='store_true',
                        help='Add inferred own-source leaf continuation strictly outside the west map edge')
    parser.add_argument('--west-native-frame', action='store_true',
                        help='Align west continuation with native boundary depth and projected vertical span')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    source, output = args.source.resolve(), args.output.resolve()
    require(not output.exists(), 'Use a fresh candidate destination')
    require(0 <= args.fragment_jitter <= 12, 'Fragment jitter must be in0..12 native pixels')
    require(not args.west_native_frame or args.west_edge_completion,
            'West native frame requires west edge completion')
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
        if args.soft_branch_envelope:
            closest_depth = -b/(2*a)
            radial_distance = np.maximum(0., c+1-b*b/(4*a))
            branch_depth = closest_depth + np.exp(-.5*radial_distance)/np.sqrt(a)
            weights = np.exp(-2*radial_distance)
            weights /= max(float(weights.sum()), 1e-300)
            depth = float(np.sum(weights*branch_depth))
        elif np.any(valid):
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
                    phase = math.sin((x0+left)*12.9898+(y0+top)*78.233)*43758.5453
                    depth += args.fragment_jitter*(2*(phase-math.floor(phase))-1)
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
        depth_scale = 1.
        if args.minimum_depth_width:
            require(1 <= args.minimum_depth_width <= 1.5, 'Depth target must be in1..1.5')
            points = np.asarray(vertices)
            depths = points @ ray
            center_depth = float((depths.min()+depths.max())/2)
            offsets = depths-center_depth
            width_target = float(np.ptp(points[:, 0]))*args.minimum_depth_width
            while np.ptp((points+(depth_scale-1)*offsets[:, None]*ray)[:, 1]) < width_target:
                depth_scale += .01
                require(depth_scale < 4, 'Unable to fit bounded inferred crown depth')
            points += (depth_scale-1)*offsets[:, None]*ray
            if points[:, 2].min() < 20:
                points += ray*((20-points[:, 2].min())/SIN)
            vertices = points.tolist()
        result = replace_mesh(crown, vertices, faces, uvs, materials, slots, known)
        require(foreign == {o.name: (geometry(o), appearance(o)) for o in bpy.data.objects
                           if o.type == 'MESH' and o != crown}, 'Foreign or wood receiver changed')
        output.mkdir(parents=True)
        west_completion = None
        if args.west_edge_completion:
            from complete_northern_caps import cap
            (output / 'inspection').mkdir()
            west_completion = cap(crown, source / 'inspection/source-packet/partition.json',
                                  int(cfg['asset_id'].rsplit('-', 1)[1]), output / 'inspection', edge='west',
                                  west_native_frame=args.west_native_frame)
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
            inferred_native_ray_depth_scale=depth_scale,
            fragment_jitter=args.fragment_jitter,
            soft_branch_envelope=args.soft_branch_envelope,
            west_edge_completion=west_completion,
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
