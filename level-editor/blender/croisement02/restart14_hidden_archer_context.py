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
from refinement_review import _tree


def native_joint_audit(objects,folder,profile,crop):
    record=json.loads((folder/'construction.json').read_text())
    rgba=np.array(Image.open(record['source']).convert('RGBA'))
    x0,y0=record['source_top_left'];h,w=rgba.shape[:2]
    source_dir=OUT/'restart7-source-patch-delivery/contracts-v1/source-review-v1'
    authority=json.loads((source_dir/'manifest.json').read_text())
    state=folder.name.rsplit('-',1)[1]
    reference=next(r for r in authority['images'] if r['profile']==profile and r['label'].lower()==state)
    image=source_dir/reference['image'];assert sha(image)==reference['sha256']
    original=np.array(Image.open(image).convert('RGBA'))
    assert reference['crop']==crop
    tree,owners,_=_tree(objects);hits={};blocked=[];native_hidden=[];expected_visible=0
    for y,x in np.argwhere(rgba[:,:,3]>=128):
        gx,gy=int(x+x0),int(y+y0)
        visible=bool(np.array_equal(original[gy-crop[1],gx-crop[0],:3],rgba[y,x,:3]))
        expected_visible+=visible
        origin=Vector((gx+.5,-(gy+.5)/SIN,0))+RAY*6000
        point,normal,index,distance=tree.ray_cast(origin,-RAY)
        owner=owners[index].get('asset_group',owners[index].name) if point is not None else '<none>'
        hits[owner]=hits.get(owner,0)+1
        ours=owner==record['asset_id']
        if visible and not ours:blocked.append(dict(pixel=[gx,gy],first_hit=owner,world=list(point) if point is not None else None))
        if not visible and ours:native_hidden.append([gx,gy])
    return dict(state=state,model_sha256=record['model_sha256'],source_reference=str(image),
        source_reference_sha256=reference['sha256'],native_source_visible_centers=expected_visible,
        first_hits=hits,blocked_native_visible_centers=blocked,
        leaf_centers_not_matching_final_source_reference=native_hidden,
        limits=['Source RGB equality identifies demonstrated sprite-color centers only; unequal composite centers retain separate native layering uncertainty.',
                'First-hit attribution is geometric, not a final shaded color or complete scene parity certificate.'])


def load_meshes(scene,path,asset,expected=None):
    before=set(bpy.data.objects)
    with bpy.data.libraries.load(str(path),link=False) as (src,dst):
        names=list(src.objects);dst.objects=src.objects
    added=[o for o in bpy.data.objects if o not in before and o.type=='MESH' and o.get('asset_group')==asset]
    assert added,asset
    for obj in added:
        parent=obj
        while parent:
            if parent.name not in scene.objects:scene.collection.objects.link(parent)
            parent=parent.parent
    bpy.context.view_layer.update()
    if expected:
        for original,obj in zip(names,dst.objects):
            if obj in added:
                assert np.max(np.abs(np.array(obj.matrix_world)-np.array(expected[original])))<1e-6,original
    return added


def main(root=None,audit_only=False,wood_contact=False):
    root=Path(root) if root else OUT/'restart14-hidden-archer/candidate-v2'
    audit=OUT/'restart14-hidden-archer/audit-v1'
    authority=json.loads((audit/'first-hit-v1/report.json').read_text())
    profiles=json.loads((audit/'source-authority.json').read_text())['profiles']
    expected={}
    for replacement in authority['replacements']:
        assert sha(replacement['model'])==replacement['model_sha256']
        bpy.ops.wm.open_mainfile(filepath=replacement['model']);bpy.context.view_layer.update()
        expected[replacement['asset']]={o.name:[list(row) for row in o.matrix_world]
            for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')==replacement['asset']}
    for number in [1,2,5]:
        destination=root/f'profile-{number:02d}-context-{"audit" if audit_only else "wood-contact-v1" if wood_contact else "v1"}'
        destination.mkdir(exist_ok=False)
        assert sha(authority['source'])==authority['source_sha256']
        bpy.ops.wm.open_mainfile(filepath=authority['source'])
        scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and not o.hide_render]
        for replacement in authority['replacements']:
            assert sha(replacement['model'])==replacement['model_sha256']
            objects=[o for o in objects if o.get('asset_group')!=replacement['asset']]
            objects+=load_meshes(scene,replacement['model'],replacement['asset'],expected[replacement['asset']])
        row=next(p for p in profiles if p['profile'].endswith(f'{number:02d}'))
        x0,y0,x1,y1=row['crop'];context=[]
        for obj in objects:
            points=np.array([obj.matrix_world@Vector(p) for p in obj.bound_box])
            screen=np.column_stack([points[:,0],-points[:,1]*SIN-points[:,2]*COS])
            low,high=screen.min(0),screen.max(0)
            if high[0]>=x0-8 and low[0]<=x1+8 and high[1]>=y0-8 and low[1]<=y1+8:context.append(obj)
        suppressed=[]
        if wood_contact:
            suppressed=[o.name for o in context if ' / Crown' in o.name]
            context=[o for o in context if o.name not in suppressed]
        for obj in scene.objects:
            if obj.type=='MESH':obj.hide_render=obj not in context
        endpoints={};pins=[]
        for state in ['initial','applied']:
            folder=root/f'profile-{number:02d}-{state}';r=json.loads((folder/'construction.json').read_text())
            assert sha(folder/'model.blend')==r['model_sha256']
            endpoints[state]=load_meshes(scene,folder/'model.blend',r['asset_id'])
            pins.append(dict(state=state,model=str(folder/'model.blend'),model_sha256=r['model_sha256']))
        dependencies={str(Path(bpy.path.abspath(lib.filepath)).resolve()):sha(Path(bpy.path.abspath(lib.filepath)).resolve())
                      for lib in bpy.data.libraries}
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
            if not audit_only:
                joint=native_joint_audit(context+endpoints[state],root/f'profile-{number:02d}-{state}',row['profile'],row['crop'])
                write_json(destination/f'{state}-native-audit.json',joint)
            if audit_only:
                result=native_joint_audit(context+endpoints[state],root/f'profile-{number:02d}-{state}',row['profile'],row['crop'])
                records.append(result)
                write_json(destination/f'{state}-native-audit.json',result)
                continue
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
            linked_dependencies=dependencies,reopened_source_world_matrices=expected,
            context_objects=[o.name for o in context],diagnostic_suppressed_crowns=suppressed,limits=[
                'Diagnostic crown suppression, when listed, changes only render visibility to expose wood/foliage contact; full-context native audit remains authoritative.',
                'Static scene uses the explicitly pinned local tree replacements; other later derivatives may be absent.',
                'Context trees may extend beyond the local view; all proposed foliage itself is fully framed.',
                'Initial/applied endpoint geometry only; no intermediate movement or runtime integration.']))
        for pin in pins:assert sha(pin['model'])==pin['model_sha256']


if __name__=='__main__':
    acquire()
    try:main(audit_only='--audit-only' in sys.argv,wood_contact='--wood-contact' in sys.argv)
    finally:release()
