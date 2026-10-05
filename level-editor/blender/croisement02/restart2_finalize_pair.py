"""Bind packaged fence/flower evidence without changing either candidate model."""
import json
import sys
from pathlib import Path

import bpy
import bmesh
import numpy as np
from PIL import Image
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
               str(ROOT / 'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from tree_geometry import SIN, RAY

WORKERS = [OUT / 'wattle99-source-candidate/packaged-v6-r2/assets/croisement02-southwest-path-wattle-fence',
           OUT / 'restart2-fence/flower76-package-v1/assets/croisement02-shrub-76']


def mask(row, parent):
    result = np.zeros((1152, 1792), bool)
    image = np.asarray(Image.open(parent / row['png']).convert('L')) > 0
    x, y = row['box_top_left']
    result[y:y+image.shape[0], x:x+image.shape[1]] = image
    return result


def main(center_rays=False):
    pair = OUT / 'wattle99-source-candidate/v6/flower-joint-completed-v1'
    evidence = json.loads((pair / 'evidence.json').read_text())
    workers = [dict(path=str(p), model_sha256=sha(p / 'model.blend')) for p in WORKERS]
    joint = OUT / 'restart2-fence/pair-binding-v1.json'
    write_json(joint, dict(status='Exact packaged preservation binds the independently reviewed pair',
                          workers=workers, original_pair_evidence=str(pair / 'evidence.json'),
                          original_pair_evidence_sha256=sha(pair / 'evidence.json'),
                          first_hit_evidence=str(pair / 'first-hit.json'), first_hit_sha256=sha(pair / 'first-hit.json'),
                          first_hit=json.loads((pair / 'first-hit.json').read_text())))
    for index, worker in enumerate(WORKERS):
        digest = sha(worker / 'model.blend')
        proof = worker / 'inspection/package-preservation.json'
        preservation = json.loads(proof.read_text())
        assert preservation['model_sha256'] == digest and preservation['exact_geometry_uv_material_preservation']
        source_key = 'fence_model_sha256' if index == 0 else 'plant_model_sha256'
        assert evidence[source_key] in preservation['protected'].values()
        bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
        objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                   if o.type == 'MESH' and o.get('asset_group') == worker.name]
        scene = bpy.context.scene
        for obj in scene.objects:
            if obj.type == 'MESH':
                obj.hide_render = obj not in objects
        manifest = json.loads((worker / 'source-masks.json').read_text())
        inventory = Path(manifest['mask_inventory'])
        masks = {r['index']: mask(r, inventory.parent)
                 for r in json.loads(inventory.read_text())['masks']}
        assignment = manifest['projections']['exterior']['assignments'][0]
        expected = np.logical_or.reduce([masks[n] for n in assignment['mask_indices']])
        excluded = np.zeros_like(expected)
        for n in assignment.get('exclude_mask_indices', []):
            excluded |= masks[n]
        expected &= ~excluded
        yy, xx = np.nonzero(expected)
        l, t, r, b = max(0, int(xx.min())-8), max(0, int(yy.min())-8), min(1792, int(xx.max())+9), min(1152, int(yy.max())+9)
        data = bpy.data.cameras.new('Exact packaged source coverage')
        data.type = 'ORTHO'
        data.sensor_fit = 'HORIZONTAL'
        data.ortho_scale = r-l
        data.clip_end = 20000
        camera = bpy.data.objects.new(data.name, data)
        scene.collection.objects.link(camera)
        center = Vector(((l+r)/2, -(t+b)/2/SIN, 0))
        camera.location = center + RAY*5000
        camera.rotation_euler = (center-camera.location).to_track_quat('-Z', 'Y').to_euler()
        scene.camera = camera
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 8
        scene.cycles.transparent_max_bounces = 128
        scene.cycles.use_denoising = False
        if center_rays:
            scene.cycles.pixel_filter_type = 'BOX'
            scene.cycles.filter_width = .01
            scene.cycles.seed = 0
            scene.cycles.use_adaptive_sampling = False
            scene.render.use_compositing = False
            scene.render.dither_intensity = 0
        scene.render.resolution_x = (r-l)*3
        scene.render.resolution_y = (b-t)*3
        scene.render.resolution_percentage = 100
        scene.render.film_transparent = True
        scene.render.image_settings.color_mode = 'RGBA'
        scene.view_settings.view_transform = 'Standard'
        scene.view_settings.look = 'None'
        dest = worker / ('inspection/source-coverage-center-rays' if center_rays else 'inspection/source-coverage')
        dest.mkdir(exist_ok=False)
        scene.render.filepath = str(dest / 'render.png')
        bpy.ops.render.render(write_still=True)
        actual = np.asarray(Image.open(dest / 'render.png').convert('RGBA'))[1::3, 1::3, 3] > 127
        actual &= ~excluded[t:b, l:r]
        want = expected[t:b, l:r]
        intersection = want & actual
        report = dict(model_sha256=digest, expected_pixels=int(want.sum()), rendered_pixels=int(actual.sum()),
                      missing_pixels=int((want & ~actual).sum()), extra_pixels=int((actual & ~want).sum()),
                      intersection_over_union=float(intersection.sum() / (want | actual).sum()),
                      source_recall=float(intersection.sum() / want.sum()), source_crop=[l, t, r, b],
                      mask_inventory_sha256=sha(inventory), source_masks_sha256=sha(worker / 'source-masks.json'),
                      sampling='Three-times native render sampled at pixel centers;128 transparent bounces.',
                      center_ray_filter='BOX0.01;seed0;adaptive/compositing/dither off' if center_rays else None,
                      semantics='Reported foliage-excluded wood comparison' if index == 0 else 'Observed502 plus separately inferred6002; native first-hit roles remain separately bound.')
        if index == 1:
            report['source_roles'] = [{ 'domain': n, 'pixels': int(masks[n].sum()),
                                       'covered': int((masks[n][t:b, l:r] & actual).sum()) } for n in [502, 6002]]
        write_json(dest / 'report.json', report)
        overlay = np.asarray(Image.open(OUT / 'animation-references/composite-frame-0.png').convert('RGB').crop((l, t, r, b))).copy()
        overlay[want & ~actual] = [255, 30, 30]
        overlay[actual & ~want] = [0, 220, 255]
        Image.fromarray(overlay).save(dest / 'difference.png')
        if center_rays:
            print(worker.name, report)
            continue
        topology = []
        for obj in objects:
            bm = bmesh.new()
            bm.from_mesh(obj.data)
            topology.append(dict(object=obj.name, vertices=len(bm.verts), faces=len(bm.faces),
                                 nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),
                                 degenerate_faces=sum(f.calc_area() < 1e-9 for f in bm.faces)))
            bm.free()
        write_json(worker / 'inspection/refinement.json', dict(asset_id=worker.name, model_sha256=digest,
                   status='Private candidate independently reviewed for geometry; user approval pending',
                   topology=topology, source_roles=assignment,
                   limitations=preservation['limitations'], preservation_sha256=sha(proof)))
        if index == 0:
            assert sum(r['nonmanifold_edges'] + r['degenerate_faces'] for r in topology) == 0
            write_json(worker / 'inspection/fence-topology.json', dict(model_sha256=digest, objects=topology))
        joint_path = worker / 'inspection/joint-neighbourhood.json'
        write_json(joint_path, dict(model_sha256=digest, evidence=str(joint), evidence_sha256=sha(joint),
                                    sheet=str(pair / 'actual-eight.png'), sheet_sha256=sha(pair / 'actual-eight.png'),
                                    preservation_sha256=sha(proof)))
        actual_sheet = worker / 'inspection/actual-materials/sheet.png'
        assert json.loads((actual_sheet.parent / 'evidence.json').read_text())['model_sha256'] == digest
        write_json(worker / 'inspection/visual-review.json', dict(
            status='Worker self-review plus independent coordinator pair geometry PASS; user approval pending',
            ready_for_geometry_review=True, model_sha256=digest, sheet_sha256=sha(actual_sheet),
            source_coverage_sha256=sha(dest / 'report.json'),
            joint_neighbourhood_sha256=sha(joint_path), preservation_evidence=str(proof),
            preservation_evidence_sha256=sha(proof),
            independent_review='Coordinator inspected source comparison and actual8: irregular woven contour matches source better and flowers remain in front. Exact packaged preservation binds those reviewed models.',
            limitations=preservation['limitations'], user_approval=False))
        assert sha(worker / 'model.blend') == digest
        print(worker.name, report)


if __name__ == '__main__':
    acquire()
    try:
        main(center_rays='--center-rays' in sys.argv)
    finally:
        release()
