"""Export a frozen complete mechanism and preserve its reviewed rigid pose coordinate space."""
import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/york-refinement/restart2'
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from render_slots import acquire, release


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main(args):
    import bpy
    from export_editor import export_asset_library
    from refinement_workspace import _geometry
    source, out = Path(args.model).resolve(), Path(args.output).resolve()
    assert source.is_relative_to(BASE) and out.is_relative_to(BASE) and source.name == 'model.blend'
    assert not out.exists() and sha(source) == args.model_sha256
    poses_path = BASE / 'restart62-winch-rigid-poses-v2/poses.json'
    assert sha(poses_path) == 'c277b27992bebb1eb0f29455ea019af1b41efa905ae7b39a59c9bf83b694330c'
    poses = json.loads(poses_path.read_text())
    names = [row['node'] for row in poses['frames'][0]]
    assert len(names) == 109 and len(set(names)) == 109
    assert shutil.disk_usage(ROOT).free >= 10 * 1024**3
    assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines()
                    if l.startswith('MemAvailable:'))) * 1024 >= 6 * 1024**3
    approved = Path(poses['source_model'])
    assert sha(approved) == poses['source_model_sha256']
    def geometry_poses():
        result = []
        for frame in range(45):
            bpy.context.scene.frame_set(frame * 2)
            bpy.context.view_layer.update()
            result.append({name: _geometry(bpy.data.objects[name]) for name in names})
        return result
    bpy.ops.wm.open_mainfile(filepath=str(approved))
    approved_geometry = geometry_poses()
    bpy.ops.wm.open_mainfile(filepath=str(source))
    assert geometry_poses() == approved_geometry, 'Frozen mechanism geometry or motion drifted'
    scene = bpy.context.scene
    scene.name = 'york Refinement'
    scene.frame_set(88)
    scene.render.threads_mode = 'FIXED'
    scene.render.threads = 2
    bpy.context.view_layer.update()
    objects = [bpy.data.objects[name] for name in names]
    assert all(o.type == 'MESH' and o.get('native_patch') == 'patch-004' for o in objects)
    before = {o.name: _geometry(o, protect_appearance=True) for o in scene.objects}
    collection = bpy.data.collections.get('york Working')
    if collection is None:
        collection = bpy.data.collections.new('york Working')
        scene.collection.children.link(collection)
    aid = 'york-castle-winch'
    old = {}
    for obj in objects:
        old[obj.name] = {k: obj[k] for k in ['asset_group', 'asset_name', 'part_name'] if k in obj}
        assert obj['source_node'] == 'scenery-york-castle-winch'
        obj['asset_group'], obj['asset_name'], obj['part_name'] = aid, 'Castle winch mechanism', 'mechanism'
        if obj.name not in collection.objects:
            collection.objects.link(obj)
    assert {o.name for o in collection.objects if o.type == 'MESH' and o.get('asset_group') == aid} == set(names)
    export = export_asset_library('york', out / '3d-assets', BASE.parent / 'baseline/york.rhp.json',
                                  standalone_pivots={aid: poses['source_origin_scene']},
                                  asset_ids=[aid], include_hidden_objects=names)
    for obj in objects:
        for key in ['asset_group', 'asset_name', 'part_name']:
            if key in old[obj.name]:
                obj[key] = old[obj.name][key]
            else:
                del obj[key]
    assert before == {o.name: _geometry(o, protect_appearance=True) for o in scene.objects}
    model = out / '3d-assets' / aid / 'model.glb'
    descriptor = model.with_name('asset.json')
    proof = {'status': 'PRIVATE_EXPORT_PENDING_APPEARANCE_APPROVAL_AND_BROWSER', 'asset_id': aid,
             'source_model_sha256': args.model_sha256, 'source_model': str(source),
             'model_sha256': sha(model), 'descriptor_sha256': sha(descriptor),
             'source_origin_scene': poses['source_origin_scene'], 'pose_data_sha256': sha(poses_path),
             'all45poses_geometry_exact': True, 'terminal_frame': 88, 'export_report': export, 'canonical_writes': False,
             'scope': 'All109 mechanism pieces only; room/gatehouse excluded. Native initial remains absent.'}
    (out / 'export-report.json').write_text(json.dumps(proof, indent=2) + '\n')
    sys.path.insert(0, str(Path(__file__).parent))
    import restart42_approved_export_surface_guard as guard
    guard.CONFIG['winch'] = (str(out.relative_to(BASE)), str(source.parent.relative_to(BASE)))
    guard.main('winch', expected_objects=names, frame=88)
    print(json.dumps(proof), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--model', required=True)
    parser.add_argument('--model-sha256', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    acquire(slots=2)
    try:
        main(args)
    finally:
        release()
