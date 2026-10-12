"""Export baked Sherwood texture candidates without changing the live library."""
import argparse
import json
import shutil
from pathlib import Path
import sys

import bpy
import numpy as np

HERE = Path(__file__).resolve().parent
EDITOR = HERE.parents[1]
sys.path[:0] = [str(HERE), str(EDITOR/'refinement'), str(EDITOR/'refinement/blender')]
from stage_editor_migration import sha, write, finalize_glb
from export_editor import export_editor
from asset_index import write_asset_index
from verify_editor_handoff import points, deviation
from lossy_assets import read_glb, accessor_array, mesh_instances, refresh_derivatives
import render_slots


def main(baseline, candidate, output, selected=None):
    baseline, candidate, output = [Path(p).resolve() for p in (baseline, candidate, output)]
    installed = json.loads((baseline/'installation.json').read_text())
    fill = json.loads((candidate/'fill.json').read_text())
    if installed['status'] != 'APPLIED' or not fill['geometry_and_uvs_unchanged']:
        raise ValueError('Missing initial publication or verified texture bake')
    worker = candidate/'candidate.blend'
    if sha(worker) != fill['worker_sha256']:
        raise ValueError('Candidate worker changed since its texture verification')
    verification=json.loads((candidate/'saved-worker-verification.json').read_text())
    if verification['status']!='PASS_SAVED_TEXTURE_CANDIDATE' or verification['worker_sha256']!=fill['worker_sha256']:
        raise ValueError('Saved texture worker has not passed source preservation checks')
    plan=json.loads((EDITOR/'work/needs-review/sherwood-refinement/grouping-review/plan.json').read_text())
    independent={g['id'] for g in plan['groups'] if not g['changed']}
    available={r['asset'] for r in fill['assets']}
    if {f'sherwood-terrain-{x}-{y}' for x in (1,2) for y in (1,2,3)} <= available:
        available.add('sherwood-terrain');independent.add('sherwood-terrain')
    selected=set(selected) if selected else available & independent
    if not selected or not selected <= available & independent:
        raise ValueError('Partial publication requires complete independently replaceable assets')
    output.mkdir(parents=True, exist_ok=False)
    render_slots.acquire()
    bpy.ops.wm.open_mainfile(filepath=str(worker))
    bpy.context.window.scene = bpy.data.scenes['Sherwood Editor Migration']
    groups = {}
    for obj in bpy.data.collections['Sherwood Working'].objects:
        if obj.type == 'MESH':
            groups.setdefault(obj['asset_group'], []).append(obj)
    live=EDITOR/'library'
    document = json.loads((live/'scenes/sherwood.rhlos-map.json').read_text())
    before={'scenes/sherwood.rhlos-map.json':sha(live/'scenes/sherwood.rhlos-map.json')}
    protected={str(candidate/'candidate.blend'):sha(candidate/'candidate.blend'),str(candidate/'fill.json'):sha(candidate/'fill.json'),str(candidate/'saved-worker-verification.json'):sha(candidate/'saved-worker-verification.json')}
    reports = []
    root = output/'map-assets/3d-assets'
    for ref in document['assetSources'] + document['sceneAssets']:
        for key in ('model','descriptor'):before[ref[key]]=sha(live/ref[key])
        descriptor = json.loads((live/ref['descriptor']).read_text())
        if ref['id'] not in selected:
            shutil.copytree((live/ref['descriptor']).parent,(output/'map-assets'/ref['descriptor']).parent)
            continue
        directory = root/'sherwood'/ref['id']
        model = directory/'model.glb'
        export_editor('Sherwood', model, asset_id=ref['id'],
                      standalone_pivot=descriptor.get('source_origin_scene', [0, 0, 0]))
        finalize_glb(model, descriptor)
        for key in ('lossy_model', 'preview_model'):
            descriptor.pop(key, None)
        descriptor.pop('legacy_refinement',None)
        descriptor['texture_candidate'] = {
            'status': 'published-at-user-request-review-pending',
            'publication_authorization':'Install completed textures in the level editor', 'worker_sha256': fill['worker_sha256'],
            'source_worker_sha256': fill['source_worker_sha256'],
            'fill_report_sha256': sha(candidate/'fill.json'),
        }
        write(directory/'asset.json', descriptor)
        ref['model_sha256'] = sha(model)
        ref['descriptor_sha256'] = sha(directory/'asset.json')
        gltf, binary, _ = read_glb(model)
        instances = mesh_instances(gltf)
        exported, triangles = [], 0
        for index, mesh in enumerate(gltf.get('meshes', [])):
            matrix = instances[index]
            for primitive in mesh['primitives']:
                vertices = accessor_array(gltf, binary, primitive['attributes']['POSITION'], True)
                exported.append(vertices @ matrix[:3, :3].T + matrix[:3, 3]
                                + descriptor.get('source_origin_scene', [0, 0, 0]))
                triangles += len(accessor_array(gltf, binary, primitive['indices'])) // 3
        expected, actual = points(groups[ref['id']]), np.concatenate(exported)
        drift = max(deviation(expected, actual), deviation(actual, expected))
        count = 0
        for obj in groups[ref['id']]:
            obj.data.calc_loop_triangles()
            count += len(obj.data.loop_triangles)
        if drift >= .001 or count != triangles:
            raise ValueError(f'Candidate export changed geometry: {ref["id"]}, {drift}, {count}/{triangles}')
        reports.append({'asset': ref['id'], 'triangles': triangles, 'maximum_world_vertex_drift': drift})
        print(json.dumps(reports[-1]), flush=True)
    write(output/'sherwood.rhlos-map.json', document)
    write_asset_index(root)
    write(output/'texture-handoff.json', {
        'status': 'PASS', 'texture_review': 'pending', 'assets': reports,
        'before':before,'protected_files':protected,'publication_authorization':'Install completed textures in the level editor',
        'source_worker_sha256': fill['source_worker_sha256'],
        'candidate_worker_sha256': fill['worker_sha256'], 'live_library_changed': False,
    })
    render_slots.release()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    from partial_texture_publication import verify
    verify(output)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--baseline', required=True)
    parser.add_argument('--candidate', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--asset',action='append')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    main(args.baseline, args.candidate, args.output,args.asset)
