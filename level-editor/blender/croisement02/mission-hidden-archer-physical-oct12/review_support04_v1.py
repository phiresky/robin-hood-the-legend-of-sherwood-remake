"""Review each saved support pose without changing the pinned model."""
import json,sys,time
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import build_models as b
from build_support04_v1 import fingerprint
from render_slots import acquire,release

def main():
    dest=b.q.OUT/'geometry-v8-short-support04-review-v1'
    assert not dest.exists()
    b.DEST=dest;b.guard();dest.mkdir()
    construction=json.loads((b.q.OUT/'geometry-v8-short-support04-v3/report.json').read_text())
    model=b.q.ROOT/construction['model']['path']
    assert b.q.pin(model)==construction['model']
    b.bpy.ops.wm.open_mainfile(filepath=str(model))
    meshes=[o for o in b.bpy.context.scene.objects if o.type=='MESH']
    assert {o['source_phase']:fingerprint(o)for o in meshes if not o.get('inferred_support')}==construction['source_mesh_fingerprints']
    reviews=[]
    for phase in ['initial','transition-0','transition-1','applied']:
        active=[o for o in meshes if o['source_phase']==phase]
        for o in meshes:o.hide_render=o not in active;o.hide_viewport=o not in active
        b.bpy.context.view_layer.update()
        points=[tuple(o.matrix_world@v.co)for o in active for v in o.data.vertices]
        mesh=b.bpy.data.meshes.new('Review framing bounds');mesh.from_pydata(points,[],[])
        proxy=b.bpy.data.objects.new('Invisible framing proxy',mesh);b.bpy.context.scene.collection.objects.link(proxy);proxy.hide_render=True
        folder=dest/phase;folder.mkdir();b.render(proxy,folder)
        b.bpy.data.objects.remove(proxy,do_unlink=True);b.bpy.data.meshes.remove(mesh)
        reviews.append(dict(phase=phase,visible_objects=[o.name for o in active],sheets=[b.q.pin(folder/f'{mode}-eight.png')for mode in ['textured','solid']]))
    assert b.q.pin(model)==construction['model']
    (dest/'report.json').write_text(json.dumps(dict(status='PRIVATE_SAVED_MODEL_REVIEW_PENDING',model=construction['model'],reviews=reviews,limits=['Wood junction context requires separate review.','Unresolved components remain unchanged; no approval readiness asserted.']),indent=2)+'\n')

if __name__=='__main__':
    def available():return int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024
    while True:
        while available()<11*1024**3:print('Waiting11GiB before saved support review',flush=True);time.sleep(30)
        acquire(slots=2)
        try:
            if available()<10*1024**3:continue
            main();break
        finally:release()
