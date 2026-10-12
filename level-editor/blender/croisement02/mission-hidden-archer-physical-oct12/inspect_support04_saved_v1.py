"""Read saved attachment geometry without importing the large receiver scene."""
import json, sys, time
from pathlib import Path
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import build_models as b
from build_support04_v1 import fingerprint
from render_slots import acquire, release

def main():
    dest = b.q.OUT / 'support04-saved-junction-readback-v1'
    assert not dest.exists()
    b.DEST = dest
    b.guard()
    construction = json.loads((b.q.OUT / 'geometry-v8-short-support04-v4/report.json').read_text())
    model = b.q.ROOT / construction['model']['path']
    assert b.q.pin(model) == construction['model']
    b.bpy.ops.wm.open_mainfile(filepath=str(model))
    meshes = [o for o in b.bpy.context.scene.objects if o.type == 'MESH']
    for obj in meshes:
        obj.hide_viewport = False
    b.bpy.context.view_layer.update()
    assert {o['source_phase']: fingerprint(o) for o in meshes if not o.get('inferred_support')} == construction['source_mesh_fingerprints']
    records = []
    for row in construction['rows']:
        if not row['status'].startswith('BUILT'):
            continue
        name = f"Inferred short support04-{row['phase']}-{row['component']}"
        obj = b.bpy.data.objects[name]
        points = [obj.matrix_world @ v.co for v in obj.data.vertices]
        center = sum(points, b.Vector()) / len(points)
        expected = (b.Vector(row['support_start']) + b.Vector(row['support_end'])) / 2
        assert (center - expected).length < .002, (name, list(center), list(expected))
        length = (b.Vector(row['support_end']) - b.Vector(row['support_start'])).length
        axis = (b.Vector(row['support_end']) - b.Vector(row['support_start'])).normalized()
        projections = [(p-center).dot(axis) for p in points]
        assert abs(max(projections) - min(projections) - length) < .002
        foliage = next(o for o in meshes if not o.get('inferred_support') and o['source_phase'] == row['phase'])
        foliage.data.calc_loop_triangles()
        local_triangles = []
        for tri in foliage.data.loop_triangles:
            verts = [foliage.matrix_world @ foliage.data.vertices[i].co for i in tri.vertices]
            if min((p-center).length for p in verts) < max(6., length+2):
                local_triangles.append(dict(vertices=[list(p) for p in verts], material_index=tri.material_index))
        records.append(dict(phase=row['phase'], component=row['component'], name=name,
            center_error=(center-expected).length, actual_length=max(projections)-min(projections),
            vertices=[list(p) for p in points], faces=[list(p.vertices) for p in obj.data.polygons],
            matrix_world=[list(r) for r in obj.matrix_world], source_foliage_triangles=local_triangles,
            construction=row))
    dest.mkdir()
    (dest/'report.json').write_text(json.dumps(dict(status='SAVED_SUPPORT_TRANSFORMS_VERIFIED',
        model=construction['model'], records=records, recipe=b.q.pin(Path(__file__))), indent=2)+'\n')
    print(json.dumps(dict(status='SAVED_SUPPORT_TRANSFORMS_VERIFIED', supports=len(records))), flush=True)

if __name__ == '__main__':
    def available():
        return int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024
    while True:
        while available() < 11*1024**3:
            print('Waiting11GiB for bounded saved geometry readback', flush=True)
            time.sleep(30)
        acquire(slots=2)
        try:
            if available() < 10*1024**3:
                continue
            main()
            break
        finally:
            release()
