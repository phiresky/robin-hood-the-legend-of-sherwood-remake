"""Saved wood source appearance and fixed before/after close inspections."""
import argparse,json,math,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from render_multiview_asset import render


def main():
    parser=argparse.ArgumentParser();parser.add_argument('index',type=int);parser.add_argument('--workspace',type=Path);parser.add_argument('--review-directory',type=Path);a=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);index=a.index;worker=a.workspace or OUT/f'restart2-wood/branch-projected-v3/assets/croisement02-tree-{index}';p=OUT/f'restart2-wood/tree{index}-branch-fitted-v{3 if index==46 and a.workspace else 2}';geometry=json.loads((p/'evidence.json').read_text());old=Path(geometry['previous_worker']);out=a.review_directory or OUT/f'restart2-wood/tree{index}-branch-review-v2';out.mkdir(exist_ok=False)
    if sha(worker/'model.blend')!=json.loads((worker/('inspection/contour-rgb-restoration.json' if (worker/'inspection/contour-rgb-restoration.json').exists() else 'inspection/boundary-candidate.json')).read_text())['model_sha256']:raise ValueError('Unfinished saved worker')
    digest=sha(worker/'model.blend');oldhash=sha(old/'model.blend');mask=np.asarray(Image.open(geometry['boundary_mask']).convert('L'))>0;yy,xx=np.nonzero(mask);box=[max(0,int(xx.min())-10),max(0,int(yy.min())-10),min(1792,int(xx.max())+11),min(1152,int(yy.max())+11)]
    bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.view_layer.update();previous=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown'];matrices={o.name:o.matrix_world.copy() for o in previous};names=list(matrices)
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update();current=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown'];transforms={o:o.matrix_world.copy() for o in current}
    points=[]
    surfaces=[BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons]) for o in current]
    for y,x in zip(yy,xx):
        origin=Vector((float(x)+.5,-(float(y)+.5)/SIN,0))+RAY*5000;hits=[s.ray_cast(origin,-RAY) for s in surfaces];hits=[h for h in hits if h[0] is not None]
        if hits:points.append(min(hits,key=lambda h:h[3])[0])
    if not points:raise ValueError('No boundary geometry')
    array=np.array(points);target=Vector((array.min(axis=0)+array.max(axis=0))/2);scale=max(75.,float(np.linalg.norm(np.ptp(array,axis=0)))*1.65)
    with bpy.data.libraries.load(str(old/'model.blend'),link=False) as (_,loaded):loaded.objects=list(names)
    previous=loaded.objects
    scenes={}
    for state,objects in [('before',previous),('after',current)]:
        scene=bpy.data.scenes.new(f'Boundary{index} {state}');bpy.context.window.scene=scene;clones=[]
        for original in objects:
            obj=original.copy();obj.parent=None;obj.matrix_world=matrices[names[objects.index(original)]] if state=='before' else transforms[original];obj.hide_render=False;obj['asset_group']=worker.name;scene.collection.objects.link(obj);clones.append(obj)
        scene.world=bpy.data.worlds.new('Neutral boundary environment');scene.world.color=(.12,.12,.12);scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256
        light_data=bpy.data.lights.new('Boundary sun','SUN');light_data.energy=2;light=bpy.data.objects.new(light_data.name,light_data);scene.collection.objects.link(light);light.rotation_euler=(.5,-.4,-.5)
        packet=json.loads((worker/'modified/views.json').read_text());packet['scene_name']=scene.name;packet['object_names']=[o.name for o in clones];packet.pop('render_object_names',None)
        for i,view in enumerate(packet['views']):
            az=math.radians(i*45);el=math.radians(25);position=target+Vector((math.sin(az)*math.cos(el),-math.cos(az)*math.cos(el),math.sin(el)))*5000;rotation=(target-position).to_track_quat('-Z','Y').to_euler();matrix=rotation.to_matrix().to_4x4();matrix.translation=position
            view.update(camera_location=list(position),camera_rotation_euler=list(rotation),camera_matrix_world=[list(r) for r in matrix],ortho_scale=scale,crop=dict(width=256,height=256))
        write_json(out/f'{state}-views.json',packet);render(out/f'{state}-views.json',out/state,modes=('solid','textured'),width=256)
        for mode in ['solid','textured']:
            sheet=Image.new('RGB',(1024,512))
            for i in range(8):sheet.paste(Image.open(out/state/f'view-{i}-{mode}.png'),((i%4)*256,(i//4)*256))
            sheet.save(out/f'{state}-{mode}.png')
        scenes[state]=(scene,light)
    scene,light=scenes['after'];bpy.context.window.scene=scene;light.hide_render=True;scene.world.color=(0,0,0);left,top,right,bottom=box;width,height=right-left,bottom-top;target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));data=bpy.data.cameras.new('Exact source contour camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=width;data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
    scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.render.filepath=str(out/'native-actual.png');bpy.ops.render.render(write_still=True,scene=scene.name)
    expected=mask[top:bottom,left:right];actual=np.asarray(Image.open(out/'native-actual.png').convert('RGBA'));source=Image.open(worker/'reference/source.png').convert('RGB').crop(box);rgb=np.asarray(source);difference=np.max(np.abs(actual[:,:,:3].astype(int)-rgb.astype(int)),axis=2);covered=actual[:,:,3]>127;report=dict(target_pixels=int(expected.sum()),actual_alpha_covered=int((expected&covered).sum()),rgb_error_max=int(difference[expected&covered].max()),rgb_error_median=float(np.median(difference[expected&covered])),rgb_error_p95=float(np.quantile(difference[expected&covered],.95)))
    composite=Image.new('RGB',(width*3,height),(100,100,100));composite.paste(source,(0,0));display=Image.new('RGBA',(width,height),(100,100,100,255));display.alpha_composite(Image.fromarray(actual));composite.paste(display.convert('RGB'),(width,0));overlay=np.array(source);overlay[expected]=[255,40,200];composite.paste(Image.fromarray(overlay),(width*2,0));composite.resize((width*12,height*4),Image.Resampling.NEAREST).save(out/'source-comparison.png')
    write_json(out/'evidence.json',dict(model=str(worker/'model.blend'),model_sha256=digest,previous_model=str(old/'model.blend'),previous_model_sha256=oldhash,boundary_mask=geometry['boundary_mask'],boundary_mask_sha256=geometry['boundary_mask_sha256'],source_crop=box,source_appearance=report,files={p.name:sha(p) for p in out.glob('*.png')},status='Private actual saved-material review evidence; no automatic visual acceptance'))
    if sha(worker/'model.blend')!=digest or sha(old/'model.blend')!=oldhash:raise ValueError('Inspection changed models')

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
