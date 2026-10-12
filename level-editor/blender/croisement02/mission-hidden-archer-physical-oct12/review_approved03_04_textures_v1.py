"""Review the actual saved candidate materials with the unchanged approved cameras."""
import json,sys,time,shutil
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import build_models as b
from bake_approved03_04_textures_v1 import BASE,sha,write,available,geometry
from render_slots import acquire,release

def main(state,folder=None):
    folder=Path(folder) if folder is not None else BASE/'baked-v1'/state;report=json.loads((folder/'bake-validation.json').read_text())
    model=folder/'model.blend';assert sha(model)==report['model_sha256'];dest=folder/'saved-review-v2';assert not dest.exists();dest.mkdir();b.DEST=dest
    b.bpy.ops.wm.open_mainfile(filepath=str(model));objects=[o for o in b.bpy.context.scene.objects if o.type=='MESH']
    assert {o.name:geometry(o) for o in objects}==report['geometry']
    phase=state[3:];active=[o for o in objects if o['source_phase']==phase]
    for o in objects:o.hide_render=o not in active;o.hide_viewport=o not in active
    b.bpy.context.view_layer.update();points=[tuple(o.matrix_world@v.co) for o in active for v in o.data.vertices]
    mesh=b.bpy.data.meshes.new('Texture review framing');mesh.from_pydata(points,[],[]);proxy=b.bpy.data.objects.new('Texture review framing',mesh);b.bpy.context.scene.collection.objects.link(proxy);proxy.hide_render=True
    b.render(proxy,dest,modes=('textured',))
    assert sha(model)==report['model_sha256']
    write(dest/'report.json',dict(status='ACTUAL_SAVED_MATERIAL_EIGHT_VIEWS_PENDING_VISUAL_REVIEW',model_sha256=sha(model),geometry_unchanged=report['geometry'],source_camera_first=True,images={str(p):sha(p) for p in dest.glob('*.png')},texture_user_approval='pending'))
    print('ARCHER_SAVED_TEXTURE_REVIEW_COMPLETE',state,flush=True)
if __name__=='__main__':
    state=sys.argv[sys.argv.index('--')+1]
    assert (BASE/'baked-v1'/state/'bake-validation.json').exists()
    while True:
        while available()<11*1024**3:print('Waiting 11 GiB for archer saved material review',flush=True);time.sleep(30)
        assert shutil.disk_usage(BASE).free>=10*1024**3
        acquire()
        try:
            if available()<10*1024**3:continue
            main(state);break
        finally:release()
