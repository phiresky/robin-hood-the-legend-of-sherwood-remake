"""Private native and oblique foliage endpoints against pinned static neighbors."""
import json, math, sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image, ImageDraw
from mathutils import Vector

HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY


def load_meshes(scene,path,asset):
    before=set(bpy.data.objects)
    with bpy.data.libraries.load(str(path),link=False) as (src,dst):dst.objects=src.objects
    added=[o for o in bpy.data.objects if o not in before and o.type=='MESH' and o.get('asset_group')==asset]
    assert added,asset
    for obj in added:scene.collection.objects.link(obj)
    bpy.context.view_layer.update()
    return added


def main():
    root=OUT/'restart8-hidden-archer-leaf-trial-v1'
    audit=OUT/'restart8-hidden-archer-receiver-audit-v1'
    authority=json.loads((audit/'current-first-hit-v1/report.json').read_text())
    profiles=json.loads((audit/'report.json').read_text())['profiles']
    for number in [3,4]:
        destination=root/f'profile-{number:02d}-context-v1'
        destination.mkdir(exist_ok=False)
        assert sha(authority['source'])==authority['source_sha256']
        bpy.ops.wm.open_mainfile(filepath=authority['source'])
        scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and not o.hide_render]
        for replacement in authority['replacements']:
            assert sha(replacement['model'])==replacement['model_sha256']
            objects=[o for o in objects if o.get('asset_group')!=replacement['asset']]
            objects+=load_meshes(scene,replacement['model'],replacement['asset'])
        row=next(p for p in profiles if p['profile'].endswith(f'{number:02d}'))
        x0,y0,x1,y1=row['crop'];context=[]
        for obj in objects:
            points=np.array([obj.matrix_world@Vector(p) for p in obj.bound_box])
            screen=np.column_stack([points[:,0],-points[:,1]*SIN-points[:,2]*COS])
            low,high=screen.min(0),screen.max(0)
            if high[0]>=x0-8 and low[0]<=x1+8 and high[1]>=y0-8 and low[1]<=y1+8:context.append(obj)
        for obj in scene.objects:
            if obj.type=='MESH':obj.hide_render=obj not in context
        endpoints={};pins=[]
        for state in ['initial','applied']:
            folder=root/f'profile-{number:02d}-{state}';r=json.loads((folder/'construction.json').read_text())
            assert sha(folder/'model.blend')==r['model_sha256']
            endpoints[state]=load_meshes(scene,folder/'model.blend',r['asset_id'])
            pins.append(dict(state=state,model=str(folder/'model.blend'),model_sha256=r['model_sha256']))
        scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.use_denoising=False
        scene.cycles.transparent_max_bounces=512
        scene.render.resolution_percentage=100;scene.render.film_transparent=False
        scene.render.image_settings.file_format='PNG'
        scene.view_settings.view_transform='Standard'
        camera=bpy.data.objects.new('Endpoint context camera',bpy.data.cameras.new('Endpoint context camera'))
        scene.collection.objects.link(camera);scene.camera=camera;camera.data.type='ORTHO'
        camera.data.clip_start=.1;camera.data.clip_end=12000
        records=[]
        for state in ['initial','applied']:
            for name,items in endpoints.items():
                for obj in items:obj.hide_render=name!=state
            center=Vector(((x0+x1)/2,-(y0+y1)/2/SIN,0))
            camera.location=center+RAY*6000
            camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
            camera.data.ortho_scale=x1-x0
            scene.render.resolution_x=(x1-x0)*4;scene.render.resolution_y=(y1-y0)*4
            native=destination/f'{state}-native.png';scene.render.filepath=str(native);bpy.ops.render.render(write_still=True)
            points=np.array([obj.matrix_world@v.co for obj in endpoints[state] for v in obj.data.vertices])
            low,high=points.min(0),points.max(0);center=Vector((low+high)/2)
            camera.data.ortho_scale=float(np.linalg.norm(high-low))*1.25
            scene.render.resolution_x=scene.render.resolution_y=320;images=[]
            for view in range(8):
                angle=view*math.pi/4;direction=Vector((COS*math.sin(angle),-COS*math.cos(angle),SIN))
                camera.location=center+direction*6000
                camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
                path=destination/f'{state}-context-{view}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True)
                images.append(Image.open(path).convert('RGB'))
            sheet=Image.new('RGB',(1280,680),(25,25,25));draw=ImageDraw.Draw(sheet)
            for view,image in enumerate(images):
                x,y=(view%4)*320,(view//4)*340;sheet.paste(image,(x,y+20));draw.text((x+8,y+3),'Original camera' if view==0 else f'View{view+1}',fill='white')
            path=destination/f'{state}-context-eight.png';sheet.save(path)
            records.append(dict(state=state,native=str(native),native_sha256=sha(native),sheet=str(path),sheet_sha256=sha(path)))
        write_json(destination/'evidence.json',dict(status='Private context review; no geometry or endpoint approval',
            source_scene=authority['source'],source_scene_sha256=authority['source_sha256'],
            replacements=authority['replacements'],endpoints=pins,crop=row['crop'],records=records,
            context_objects=[o.name for o in context],limits=[
                'Static scene uses the explicitly pinned local tree replacements; other later derivatives may be absent.',
                'Context trees may extend beyond the local view; all proposed foliage itself is fully framed.',
                'Initial/applied endpoint geometry only; no intermediate movement or runtime integration.']))
        for pin in pins:assert sha(pin['model'])==pin['model_sha256']


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
