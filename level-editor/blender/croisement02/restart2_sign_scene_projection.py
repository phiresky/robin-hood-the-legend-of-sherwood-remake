"""Native-camera sign proof against every overlapping receiver in a frozen physical scene."""
import json,sys,math
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT,tree_workspace,scenery_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from sign_context_import import append_verified
from sign_object_mask import setup as setup_mask
from approved_texture_stage import geometry

NEIGHBORS={4:['east-stone-wall-and-gate','east-rail-fence','shrub-74'],5:['southwest-field-wattle-fence','shrub-77'],6:['north-woodland-bank','west-shrub-bank',*range(24,30)],7:['north-woodland-bank','west-rock-outcrop','southwest-rock-outcrop','northwest-boundary-shrub-54','shrub-57',2],8:['north-woodland-bank',14,15,16,17,18]}
DEST=OUT/'restart2-state/sign-scene-projection-v1'

def checked(path,digest):
    path=Path(path);assert sha(path)==digest,path;return path

def bounds(obj):
    corners=[obj.matrix_world@Vector(v) for v in obj.bound_box]
    return(min(p.x for p in corners),min(-p.y*SIN-p.z*COS for p in corners),max(p.x for p in corners),max(-p.y*SIN-p.z*COS for p in corners))

def intersects(a,b):return a[0]<b[2]and a[2]>b[0]and a[1]<b[3]and a[3]>b[1]
def refs(objects):return {o.name:dict(matrix_world=[list(r)for r in o.matrix_world],geometry=geometry(o))for o in objects}

def prepare():
    DEST.mkdir(parents=True,exist_ok=False)
    frozen=OUT/'restart2-textures/approved6-ground-scene-v1';assembly=json.loads((frozen/'assembly.json').read_text());selection_path=Path(assembly['selection']);selection=json.loads(selection_path.read_text());records={r['asset_id']:r for r in selection['records']};changes={}
    for key in dict.fromkeys(k for v in NEIGHBORS.values()for k in v):
        aid=f'croisement02-tree-{key:02}'if isinstance(key,int)else'croisement02-'+key
        worker=tree_workspace(key)if isinstance(key,int)else scenery_workspace(aid)
        if key=='north-woodland-bank':worker=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank'
        model=worker/'model.blend';digest=sha(model)
        if key!='north-woodland-bank'and records[aid]['geometry_model_sha256']==digest:continue
        audit=worker/'inspection/saved-model-audit.json'
        if audit.exists():
            a=json.loads(audit.read_text());assert a['status']=='PASS'and a['model_sha256']==digest;names=[r['object']for r in a['objects']]
        else:names=json.loads((worker/'modified/views.json').read_text())['object_names']
        changes[aid]=dict(model=str(model),model_sha256=digest,objects=names)
    sign=OUT/'state-sign-candidate/five-instances-v3/model.blend';signreport=json.loads((sign.parent/'assembly.json').read_text());checked(sign,signreport['model_sha256'])
    payload=dict(frozen_model=str(frozen/'scene.blend'),frozen_model_sha256=assembly['model_sha256'],frozen_assembly_sha256=sha(frozen/'assembly.json'),selection_sha256=sha(selection_path),changes=changes,sign_model=str(sign),sign_model_sha256=signreport['model_sha256'],limitations=['Frozen non-neighbor selections retain their explicitly older geometry/appearance.','Updated neighbors use current geometry workers; rear appearance may remain unfilled.','Canonical shrub57 retained; private sign-contact correction is not selected.','Physical depth and native draw order are assessed separately.'])
    write_json(DEST/'inputs.json',payload)

def main():
    inputs=json.loads((DEST/'inputs.json').read_text());base=checked(inputs['frozen_model'],inputs['frozen_model_sha256']);sign=checked(inputs['sign_model'],inputs['sign_model_sha256']);assembly=json.loads((sign.parent/'assembly.json').read_text());targets={r['target_index']:r for r in assembly['instances']};boxes={i:(r['native_target']['position_x']-48,r['native_target']['position_y']-64,r['native_target']['position_x']+48,r['native_target']['position_y']+32)for i,r in targets.items()}
    acquire()
    try:
        replacements={}
        for aid,row in inputs['changes'].items():
            model=checked(row['model'],row['model_sha256']);bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();replacements[aid]=refs([bpy.data.objects[n]for n in row['objects']])
        bpy.ops.wm.open_mainfile(filepath=str(base));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();collection=bpy.data.collections['Croisement02 Working'];selected=[o for o in collection.all_objects if o.type=='MESH'and not o.hide_render and o.get('asset_group')not in inputs['changes']and any(intersects(bounds(o),b)for b in boxes.values())];frozen_refs=refs(selected)
        bpy.ops.wm.open_mainfile(filepath=str(sign));scene=bpy.context.scene;bpy.context.view_layer.update();neighbors=[];imports=[]
        objects,receipt=append_verified(scene,base,list(frozen_refs),frozen_refs);neighbors.extend(objects);imports.extend(receipt)
        for aid,row in inputs['changes'].items():
            objects,receipt=append_verified(scene,checked(row['model'],row['model_sha256']),row['objects'],replacements[aid]);neighbors.extend(objects);imports.extend(receipt)
        for obj in neighbors:
            expected=next((r[obj.name]['geometry']for r in [frozen_refs,*replacements.values()]if obj.name in r),None)
            # Names may receive a suffix on import; transform verification above remains authoritative.
            if expected is not None:assert geometry(obj)==expected
        write_json(DEST/'evaluated-imports.json',dict(inputs_sha256=sha(DEST/'inputs.json'),frozen_receivers=frozen_refs,replacements=replacements,imports=imports))
        scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.use_denoising=False;scene.cycles.pixel_filter_type='BOX';scene.cycles.filter_width=.01;scene.cycles.seed=0;scene.cycles.use_adaptive_sampling=False;scene.cycles.transparent_max_bounces=1024;scene.render.dither_intensity=0;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA';scene.render.resolution_x=scene.render.resolution_y=288;scene.render.resolution_percentage=100;scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
        data=bpy.data.cameras.new('Native full-scene sign crop');data.type='ORTHO';data.ortho_scale=96;data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera
        sign_objects=[scene.objects[n]for row in targets.values()for n in row['parts']];bodies=[o for o in sign_objects if 'native_body_frame'in o];shadows=[o for o in sign_objects if 'native_frame'in o];setup_mask(scene,bodies,neighbors)
        source=Image.open(OUT/'baseline/covered.png').convert('RGBA');frames=next(p for p in json.loads((OUT/'state-target-evidence/manifest.json').read_text())['profiles']if p['id']=='TG_Panel-12')['rows'][0]['frames'];order=json.loads((OUT/'state-sign-candidate/native-order-reference-v3/manifest.json').read_text());animations=json.loads((OUT/'animation-references/manifest.json').read_text())['animations'];results=[]
        def render(path):scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);return Image.open(path).convert('RGBA')
        for index,row in targets.items():
            dest=DEST/f'target-{index}';dest.mkdir();box=boxes[index];x,y=row['native_target']['position_x'],row['native_target']['position_y'];center=Vector((x,-(y-16)/SIN,0));camera.location=center+RAY*6000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();sheet=Image.new('RGB',(864,1248),(45,45,45));phases=[]
            for ordinal,phase in enumerate([0,8,16,24]):
                scene.frame_set(1+phase*2);scene.render.use_compositing=False;actual=render(dest/f'phase-{phase:02}-actual.png');native=source.crop(box).resize((288,288),Image.Resampling.NEAREST);f=frames[phase];im=Image.open(f['image']).convert('RGBA');native.alpha_composite(im.resize((im.width*3,im.height*3),Image.Resampling.NEAREST),((48+int(f['offset'][0]))*3,(64+int(f['offset'][1]))*3))
                orderrow=next(r for r in order['records']if r['target_index']==index)
                for overlay in orderrow['overlapping_animations']:
                    assert overlay['after_sign'];f=next(a for a in animations if a['index']==overlay['index'])['frames'][0];fx,fy,fw,fh=f['bbox'];im=Image.open(f['image']).convert('RGBA').resize((fw*3,fh*3),Image.Resampling.NEAREST);native.alpha_composite(im,((fx-box[0])*3,(fy-box[1])*3))
                scene.render.use_compositing=True
                for obj in shadows:obj.hide_render=True
                joint=np.asarray(render(dest/f'phase-{phase:02}-joint.png'))[1::3,1::3]
                for obj in neighbors:obj.hide_render=True
                alone=np.asarray(render(dest/f'phase-{phase:02}-alone.png'))[1::3,1::3]
                for obj in neighbors:obj.hide_render=False
                for obj in shadows:obj.hide_render=False
                a=alone[:,:,0]>127;j=joint[:,:,0]>127;phases.append(dict(phase=phase,body_pixels=int(a.sum()),occluded_body_pixels=int((a&~j).sum())))
                sheet.paste(native.convert('RGB'),(0,ordinal*312));sheet.paste(actual,(288,ordinal*312),actual);mask=Image.fromarray(np.where((a&~j)[:,:,None],np.array([255,50,60],dtype=np.uint8),np.array([25,25,25],dtype=np.uint8))).resize((288,288),Image.Resampling.NEAREST);sheet.paste(mask,(576,ordinal*312));ImageDraw.Draw(sheet).text((4,ordinal*312+292),f'Native art | physical full-scene crop | body occluded; phase {phase}',fill='white')
            sheet.save(dest/'comparison.png');results.append(dict(target_index=index,phases=phases,comparison_sha256=sha(dest/'comparison.png')))
        checked(base,inputs['frozen_model_sha256']);checked(sign,inputs['sign_model_sha256']);write_json(DEST/'report.json',dict(status='Private physical projection evidence; native ordering mismatch and context review remain explicit',inputs_sha256=sha(DEST/'inputs.json'),evaluated_imports_sha256=sha(DEST/'evaluated-imports.json'),results=results,all_original_scene_receivers_overlapping_native_crops_included=True,camera=dict(direction=list(RAY),elevation_degrees=35),limitations=inputs['limitations']))
    finally:release()
if __name__=='__main__':
    if '--prepare'in sys.argv:prepare()
    else:main()
