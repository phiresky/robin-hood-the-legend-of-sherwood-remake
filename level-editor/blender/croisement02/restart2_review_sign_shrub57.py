"""Reopened private shrub bend: full sign poses and physical neighboring solids."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from opacity_bounds import measure
from restart2_sign_neighbors import camera_to,render,NEIGHBORS
from sign_object_mask import setup
from sign_context_import import append_verified

def main(version=1, proof_name="joint-proof", include_keys=None, obliques=True):
    base=OUT/f'restart2-fence/shrub57-sign-bend-v{version}';dest=base/proof_name;dest.mkdir(exist_ok=False)
    proof=json.loads((OUT/'restart2-fence/sign-neighbors-v4/manifest.json').read_text())
    candidate=base/'model.blend';digest=sha(candidate)
    bpy.ops.wm.open_mainfile(filepath=str(candidate));obj=bpy.data.objects['West Rock Foliage 57'];opacity=measure(obj)
    assembly=OUT/'state-sign-candidate/five-instances-v3';model=assembly/'model.blend'
    assert sha(model)==proof['sign_model_sha256']
    row=next(r for r in json.loads((assembly/'assembly.json').read_text())['instances'] if r['target_index']==7)
    bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
    selected=set(row['parts'])
    for o in list(scene.objects):
        if o.type=='MESH' and o.name not in selected:bpy.data.objects.remove(o,do_unlink=True)
    neighbors=[];bank=[];transform_receipts={}
    transform_path=OUT/'restart2-fence/sign-context-evaluated-transforms-v1.json';transform_reference=json.loads(transform_path.read_text())['inputs']
    for key in (NEIGHBORS[7] if include_keys is None else include_keys):
        info=proof['inputs'][str(key)];path=candidate if key=='shrub-57' else Path(info['worker'])/'model.blend'
        if key!='shrub-57':assert sha(path)==info['model_sha256']
        assert transform_reference[str(key)]['model_sha256']==info['model_sha256']
        imported,transform_receipts[str(key)]=append_verified(scene,path,info['objects'],transform_reference[str(key)]['objects'])
        for o in imported:
            neighbors.append(o)
            if key in ['north-woodland-bank','west-rock-outcrop','southwest-rock-outcrop']:bank.append(o)
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=128
    scene.cycles.pixel_filter_type='BOX';scene.cycles.filter_width=.01;scene.cycles.seed=0;scene.cycles.use_adaptive_sampling=False
    scene.render.film_transparent=True;scene.render.use_compositing=False;scene.render.dither_intensity=0
    scene.render.resolution_x=288;scene.render.resolution_y=288;scene.render.resolution_percentage=100
    scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    bodies=[scene.objects[n] for n in selected if 'native_body_frame' in scene.objects[n]]
    shadows=[scene.objects[n] for n in selected if 'native_frame' in scene.objects[n]]
    setup(scene,bodies,neighbors)
    data=bpy.data.cameras.new('Private complete sign bend proof');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=96;data.clip_end=20000
    camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera
    camera_to(camera,Vector((75,-270/SIN,0)),RAY)
    masks=[];sheet=Image.new('RGB',(4*288,8*312),(60,60,60))
    for phase in range(32):
        scene.frame_set(1+phase*2)
        scene.render.use_compositing=False
        for o in shadows:o.hide_render=False
        actual=render(scene,dest/f'pose-{phase:02}-actual.png')
        sheet.paste(actual,(phase%4*288,phase//4*312),actual.getchannel('A'));ImageDraw.Draw(sheet).text((phase%4*288+4,phase//4*312+290),f'Physical pose {phase}',fill='white')
        for o in shadows:o.hide_render=True
        scene.render.use_compositing=True
        joint=np.array(render(scene,dest/f'pose-{phase:02}-joint.png'))[:,:,0]>127
        for o in neighbors:o.hide_render=True
        alone=np.array(render(scene,dest/f'pose-{phase:02}-alone.png'))[:,:,0]>127
        for o in neighbors:o.hide_render=False
        masks.append(dict(phase=phase,body_pixels=int(alone[1::3,1::3].sum()),blocked_pixels=int((alone&~joint)[1::3,1::3].sum())))
    sheet.save(dest/'all32-physical.png')
    scene.frame_set(1);scene.render.use_compositing=False
    for o in shadows:o.hide_render=False
    center=Vector(row['world_anchor'])+Vector((0,0,22));data.ortho_scale=140
    sheet=Image.new('RGB',(4*288,2*312),(60,60,60))
    for i in (range(8) if obliques else []):
        angle=i*math.pi/4;camera_to(camera,center,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)))
        actual=render(scene,dest/f'oblique-{i}.png');sheet.paste(actual,(i%4*288,i//4*312),actual.getchannel('A'))
    sheet.save(dest/'joint-eight.png')
    assert sha(candidate)==digest
    write_json(dest/'report.json',dict(status='Private candidate physical proof; independent visual review pending',model_sha256=digest,sign_model_sha256=sha(model),neighbor_manifest_sha256=sha(OUT/'restart2-fence/sign-neighbors-v4/manifest.json'),context_transform_receipt_sha256=sha(transform_path),verified_context_transforms=transform_receipts,context_scope=NEIGHBORS[7] if include_keys is None else include_keys,opacity_bounds=opacity,poses=masks,total_blocked=sum(r['blocked_pixels'] for r in masks),limitations=['Known lower fringe retains its anchor; exact opaque shrub/rock intersection comparison remains pending.','Source raster changed in initial comparison; native RGBA images and UV are unchanged but that does not prove identical first-hit output.','New geometry does not inherit approval.']))
    print(dest)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
