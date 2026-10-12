"""Deferred Blender builder for Tree18 CPU wood; no context import or materials assigned."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--geometry', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    root = Path(__file__).resolve().parents[3]
    sys.path.insert(0, str(root / 'level-editor/refinement'))
    from render_slots import acquire, release
    import bpy
    import bmesh
    import numpy as np

    def resources():
        assert shutil.disk_usage(root).free >= 10 * 1024**3 + 32 * 1024**2
        mem = {r.split(':')[0]: int(r.split()[1]) * 1024 for r in Path('/proc/meminfo').read_text().splitlines()}
        assert mem['MemAvailable'] >= 6 * 1024**3

    resources()
    acquire(slots=2)
    try:
        resources()
        data = json.loads(args.geometry.read_text())
        assert sha(Path(data['source'])) == data['source_sha256']
        assert not args.output.exists()
        args.output.mkdir(parents=True)
        bpy.ops.wm.read_factory_settings(use_empty=True)
        bpy.context.preferences.filepaths.save_version = 0
        bpy.context.scene.render.threads_mode = 'FIXED'
        bpy.context.scene.render.threads = 2
        for part in data['parts']:
            name = 'Provisional Tree18 ' + part['track']
            mesh = bpy.data.meshes.new(name)
            mesh.from_pydata(part['vertices'], [], part['faces'])
            mesh.update()
            obj = bpy.data.objects.new(name, mesh)
            bpy.context.scene.collection.objects.link(obj)
            obj['tree18_track'] = part['track']
            obj['source_ownership_assigned'] = False
            obj['native_obstacle_contexts'] = [33, 34]
            obj['status'] = 'Private inferred wood; joints, roots and leaf priority require context review'
            bm = bmesh.new()
            bm.from_mesh(mesh)
            assert all(e.is_manifold for e in bm.edges) and bm.calc_volume() > 0
            bm.free()
        model = args.output / 'worker.blend'
        bpy.ops.wm.save_as_mainfile(filepath=str(model))
        assert model.stat().st_size < 8 * 1024**2
        bpy.ops.wm.open_mainfile(filepath=str(model))
        errors = []
        for part in data['parts']:
            obj = next(o for o in bpy.context.scene.objects if o.get('tree18_track') == part['track'])
            actual = np.array([list(obj.matrix_world @ v.co) for v in obj.data.vertices])
            errors.append(float(np.max(np.abs(actual - part['vertices']))))
            assert [list(p.vertices) for p in obj.data.polygons] == part['faces']
        assert max(errors) < 1e-4
        report = {'status': 'HOLD_SAVED_PARTS_NO_CONTEXT_OR_JOINT_REVIEW',
                  'geometry_sha256': sha(args.geometry), 'model_sha256': sha(model),
                  'saved_vertex_max_error': max(errors), 'materials_assigned': False,
                  'source_ownership_assigned': False, 'native_animation_imported': False,
                  'limitations': data['assumptions'] + data['contact_plan']}
        (args.output / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    finally:
        release()


if __name__ == '__main__':
    main()
