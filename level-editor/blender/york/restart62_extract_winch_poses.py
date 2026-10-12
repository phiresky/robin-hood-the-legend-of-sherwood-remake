"""Extract reviewed rigid node poses without changing their saved appearance."""
import hashlib
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/active/york/restart2'
OUT = BASE / 'restart62-winch-rigid-poses-v2'
sys.path[:0] = [str(ROOT / 'level-editor/refinement')]
from render_slots import acquire, release


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    import bpy
    from mathutils import Matrix
    source = BASE / 'restart59-approved-winch-assembly-v1/model.blend'
    assert sha(source) == 'dd3177aa0639bebadcf99a954f92a3908db2d69a681e2ae48b5592beddbed33f'
    assert not OUT.exists()
    assert shutil.disk_usage(ROOT).free >= 10 * 1024**3
    assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines()
                    if l.startswith('MemAvailable:'))) * 1024 >= 6 * 1024**3
    stable = json.loads((BASE / 'approved-texture-inputs-v1/stable24/input-review.json').read_text())['object_names']
    hardware = json.loads((BASE / 'restart60-winch-chain-texture-inputs-v1/input-review.json').read_text())['object_names']
    names = sorted(stable + hardware)
    assert len(names) == len(set(names)) == 109
    bpy.ops.wm.open_mainfile(filepath=str(source))
    scene = bpy.context.scene
    scene.render.threads_mode = 'FIXED'
    scene.render.threads = 2
    objects = [bpy.data.objects[name] for name in names]
    scene.frame_set(88)
    bpy.context.view_layer.update()
    baseline = {}
    bounds = []
    for obj in objects:
        evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
        vertices = [v.co.copy() for v in evaluated.data.vertices]
        baseline[obj.name] = (obj.matrix_world.copy(), vertices, [tuple(p.vertices) for p in evaluated.data.polygons])
        bounds.extend(obj.matrix_world @ v for v in vertices)
    pivot = [(min(v[i] for v in bounds) + max(v[i] for v in bounds)) / 2 for i in range(2)] + [min(v.z for v in bounds)]
    local = Matrix.Translation([-x for x in pivot])
    frames = []
    max_error = 0
    moving = set()
    for frame in range(45):
        scene.frame_set(frame * 2)
        bpy.context.view_layer.update()
        rows = []
        for obj in objects:
            original, vertices, faces = baseline[obj.name]
            evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
            assert [tuple(p.vertices) for p in evaluated.data.polygons] == faces, obj.name
            assert len(evaluated.data.vertices) == len(vertices), obj.name
            assert all((v.co - p).length < 1e-7 for v, p in zip(evaluated.data.vertices, vertices)), obj.name + ': nonrigid geometry'
            delta = (Matrix.Identity(4) if obj.matrix_world == original else
                     local @ obj.matrix_world @ original.inverted() @ local.inverted())
            # Exported piece vertices are baked into this common Z-up pivot space.
            # glTF's existing map wrapper owns Z-up to Y-up conversion.
            for v in vertices:
                actual = delta @ (local @ original @ v)
                expected = local @ obj.matrix_world @ v
                error = (actual - expected).length
                max_error = max(max_error, error)
                assert error < 0.003, (obj.name, frame, error)
            if any(abs(delta[r][c] - (1 if r == c else 0)) > 1e-5 for r in range(4) for c in range(4)):
                moving.add(obj.name)
            rows.append({'node': obj.name, 'matrix': [delta[r][c] for c in range(4) for r in range(4)]})
        frames.append(rows)
    OUT.mkdir()
    report = {
        'status': 'EXACT_REVIEWED_RIGID_POSES_PENDING_TEXTURE_EXPORT_AND_RUNTIME',
        'source_model': str(source), 'source_model_sha256': sha(source),
        'patch_id': 'patch-004', 'initial': 'absent', 'applied_export_frame': 88,
        'source_origin_scene': pivot, 'node_count': len(names), 'pose_count': 45,
        'moving_nodes': sorted(moving), 'max_world_vertex_error': max_error,
        'matrix_layout': 'column-major; local Z-up piece coordinates below existing map wrapper',
        'frames': frames, 'all_evaluated_local_vertices_and_topology_invariant': True,
        'canonical_writes': False,
        'scope': 'Geometry only. Final approved texture export must prove the same baked vertices and node identities.'}
    (OUT / 'poses.json').write_text(json.dumps(report, separators=(',', ':')) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k != 'frames'}), flush=True)


if __name__ == '__main__':
    acquire(slots=2)
    try:
        main()
    finally:
        release()
