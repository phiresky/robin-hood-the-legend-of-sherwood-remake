"""Review the private root correction against the exact bank and saved materials."""
import sys, json, math
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from PIL import Image, ImageDraw
ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from tree_geometry import SIN, COS, RAY
from restart2_sign_neighbors import camera_to, render
from sign_context_import import append_verified


def configure(scene):
    scene.render.engine='CYCLES'; scene.cycles.samples=8
    scene.cycles.use_denoising=False; scene.cycles.transparent_max_bounces=128
    scene.cycles.pixel_filter_type='BOX'; scene.cycles.filter_width=.01
    scene.cycles.seed=0; scene.cycles.use_adaptive_sampling=False
    scene.render.film_transparent=True; scene.render.use_compositing=False
    scene.render.dither_intensity=0; scene.render.resolution_percentage=100
    scene.render.resolution_x=384; scene.render.resolution_y=384
    scene.render.image_settings.color_mode='RGBA'
    scene.view_settings.view_transform='Standard'; scene.view_settings.look='None'
    data=bpy.data.cameras.new('Root contact review'); data.type='ORTHO'
    data.sensor_fit='HORIZONTAL'; data.clip_end=20000
    camera=bpy.data.objects.new(data.name,data); scene.collection.objects.link(camera)
    scene.camera=camera
    return camera


def main(variant='depth-v1'):
    base=OUT/'restart3-tree06-root'/variant; dest=base/'review'
    dest.mkdir(exist_ok=False)
    report=json.loads((base/'report.json').read_text())
    bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
    assert sha(bank)=='69ecb7b704e30d6d64565a44aa810a21b924195609dbe7ac35818a0209137641'
    bpy.ops.wm.open_mainfile(filepath=str(bank)); bpy.context.view_layer.update()
    bank_names=[o.name for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank']
    assert bank_names
    reference={n:dict(matrix_world=[list(r) for r in bpy.data.objects[n].matrix_world]) for n in bank_names}
    native=[]; receipts=[]
    for tag,model in [('before',Path(report['input_model'])),('after',base/'model.blend')]:
        bpy.ops.wm.open_mainfile(filepath=str(model)); bpy.context.view_layer.update()
        scene=bpy.context.scene
        objects=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-06']
        for o in scene.objects:
            if o.type=='MESH': o.hide_render=o not in objects
        camera=configure(scene); camera.data.ortho_scale=112
        camera_to(camera,Vector((650,-520/SIN,0)),RAY)
        native.append(render(scene,dest/f'native-{tag}.png'))
        neighbors,receipt=append_verified(scene,bank,bank_names,reference)
        receipts.append(dict(version=tag,imports=receipt))
        render(scene,dest/f'native-bank-{tag}.png')
        if tag=='after':
            sheet=Image.new('RGB',(1536,816),(65,65,65))
            for o in neighbors:o.hide_render=True
            bounds=np.array([tuple(o.matrix_world@v.co) for o in objects for v in o.data.vertices])
            center=Vector((bounds.min(0)+bounds.max(0))/2);camera.data.ortho_scale=float(max(np.ptp(bounds,axis=0)))*1.35
            for i in range(8):
                angle=i*math.pi/4
                camera_to(camera,center,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)))
                pic=render(scene,dest/f'actual-{i}.png');sheet.paste(pic,(i%4*384,i//4*408),pic.getchannel('A'))
                ImageDraw.Draw(sheet).text((i%4*384+5,i//4*408+388),f'Saved material view {i}',fill='white')
            sheet.save(dest/'actual-eight.png')
            for o in neighbors:o.hide_render=False
            sheet=Image.new('RGB',(1536,408),(65,65,65)); camera.data.ortho_scale=112
            center=Vector((650,(-535-COS*47)/SIN,47))
            for i in range(4):
                angle=i*math.pi/2;camera_to(camera,center,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)))
                pic=render(scene,dest/f'contact-{i}.png');sheet.paste(pic,(i*384,0),pic.getchannel('A'))
            sheet.save(dest/'contact-four.png')
    pair=Image.new('RGB',(1152,408),(65,65,65))
    source=Image.open(OUT/'baseline/covered.png').convert('RGBA').crop((594,464,706,576)).resize((384,384),Image.Resampling.NEAREST)
    for i,pic in enumerate([source,Image.open(dest/'native-bank-before.png').convert('RGBA'),Image.open(dest/'native-bank-after.png').convert('RGBA')]):
        pair.paste(pic,(i*384,0),pic.getchannel('A'))
        ImageDraw.Draw(pair).text((i*384+5,388),['Native source','Approved before / bank','Private after / bank'][i],fill='white')
    pair.save(dest/'source-comparison.png')
    a,b=[np.asarray(im).astype(int) for im in native]
    write_json(dest/'report.json',dict(model_sha256=sha(base/'model.blend'),bank_sha256=sha(bank),context_imports=receipts,
        native_isolated_changed_pixels=int(np.any(a!=b,axis=2).sum()),native_max_channel_error=int(np.abs(a-b).max()),
        native_view_first=True,status='Saved-material views complete; visual and native coverage assessment pending'))
    print(dest,flush=True)


if __name__=='__main__':
    acquire()
    try:main(sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'depth-v1')
    finally:release()
