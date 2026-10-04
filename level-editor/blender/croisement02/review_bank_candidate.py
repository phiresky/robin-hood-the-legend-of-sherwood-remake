"""Source-camera and private context checks for the isolated terrain bank."""
import json
import math
import sys
import uuid
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from prepare_bank_candidate import DEST,WORKER,ASSET,OUT,SIN,COS
from tree_geometry import RAY
from evidence_io import sha,write_json
from render_slots import acquire,release


def camera(scene, target, direction, width, height, scale):
    data=bpy.data.cameras.new('Bank review camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=scale;data.clip_end=20000
    obj=bpy.data.objects.new(data.name,data);scene.collection.objects.link(obj);obj.location=target+direction*5000;obj.rotation_euler=(target-obj.location).to_track_quat('-Z','Y').to_euler();scene.camera=obj
    scene.render.engine='CYCLES';scene.cycles.samples=4;scene.cycles.transparent_max_bounces=64
    scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100
    scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.film_transparent=True
    scene.view_settings.view_transform='Standard';scene.view_settings.look='None'


def raster(objects):
    depth=np.full((1152,1792),-np.inf);owner=np.full((1152,1792),-1,dtype=np.int16)
    names=[]
    for label,obj in enumerate(objects):
        names.append(obj['source_node']);obj.data.calc_loop_triangles()
        for triangle in obj.data.loop_triangles:
            p=np.array([obj.matrix_world@obj.data.vertices[i].co for i in triangle.vertices])
            x=p[:,0];y=-p[:,1]*SIN-p[:,2]*COS;d=-p[:,1]*COS+p[:,2]*SIN
            area=(x[1]-x[0])*(y[2]-y[0])-(x[2]-x[0])*(y[1]-y[0])
            if abs(area)<1e-8:continue
            x0=max(0,int(np.ceil(x.min()-.5)));x1=min(1791,int(np.floor(x.max()-.5)))
            y0=max(0,int(np.ceil(y.min()-.5)));y1=min(1151,int(np.floor(y.max()-.5)))
            if x1<x0 or y1<y0:continue
            gx=np.arange(x0,x1+1)+.5;gy=(np.arange(y0,y1+1)+.5)[:,None]
            a=((x[1]-gx)*(y[2]-gy)-(x[2]-gx)*(y[1]-gy))/area
            b=((x[2]-gx)*(y[0]-gy)-(x[0]-gx)*(y[2]-gy))/area;c=1-a-b
            z=a*d[0]+b*d[1]+c*d[2];win=depth[y0:y1+1,x0:x1+1]
            take=(a>=-1e-8)&(b>=-1e-8)&(c>=-1e-8)&(z>win)
            win[take]=z[take];owner[y0:y1+1,x0:x1+1][take]=label
    return owner,names


def main():
    model_hash=sha(WORKER/'model.blend')
    out=DEST/'integration'
    if out.exists():out.rename(out.with_name('integration-archive-'+uuid.uuid4().hex[:8]))
    out.mkdir()
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(WORKER/'model.blend'))
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==ASSET]
        scene=bpy.data.scenes.new('Isolated bank source camera')
        for obj in objects:
            copy=obj.copy();copy.parent=None;copy.matrix_world=obj.matrix_world.copy();copy.hide_render=False;scene.collection.objects.link(copy)
        camera(scene,Vector((896,-576/SIN,0)),RAY,1792,1152,1792)
        scene.render.filepath=str(out/'source-camera.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        # Keep the current shared assembly untouched; replace only bank0–4 in a
        # new copy. Authored shrubs and ongoing texture candidates are separate.
        bpy.ops.wm.open_mainfile(filepath=str(OUT/'integration-review/scene.blend'))
        bpy.context.preferences.filepaths.save_version=0
        scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        collection=bpy.data.collections['Croisement02 Working']
        for obj in list(collection.all_objects):
            if obj.type=='MESH' and obj.get('source_node') in {f'building-{i:03}' for i in range(5)}:bpy.data.objects.remove(obj,do_unlink=True)
        names=[row['object'] for row in json.loads((WORKER/'inspection/saved-model-audit.json').read_text())['objects']]
        with bpy.data.libraries.load(str(WORKER/'model.blend'),link=False) as (source,target):target.objects=names
        for obj in target.objects:
            collection.objects.link(obj)
        bpy.context.view_layer.update()
        for obj in target.objects:
            matrix=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False
        receivers=[o for o in collection.all_objects if o.type=='MESH' and (o.get('source_node')=='ground' or o.get('asset_group')==ASSET)]
        from mathutils.bvhtree import BVHTree
        triangles=[];vertices=[]
        for obj in receivers:
            if obj.get('source_node')=='ground':continue
            start=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices)
            obj.data.calc_loop_triangles();triangles.extend(tuple(start+i for i in t.vertices) for t in obj.data.loop_triangles)
        surface=BVHTree.FromPolygons(vertices,triangles,all_triangles=True)
        groups={}
        for obj in collection.all_objects:
            if obj.type=='MESH' and 'tree-' in obj.get('asset_group','') and obj.get('projection_component')!='crown':
                groups.setdefault(obj['asset_group'],[]).extend(obj.matrix_world@v.co for v in obj.data.vertices)
        contacts=[]
        for name,points in groups.items():
            low=min(p.z for p in points);base=[p for p in points if p.z<=low+.1]
            center=sum(base,Vector())/len(base)
            hit,normal,face,distance=surface.ray_cast(Vector((center.x,center.y,2000)),Vector((0,0,-1)))
            if hit is not None and hit.z>low+2:
                contacts.append(dict(asset=name,base_center=list(center),bank_top_z=hit.z,burial_depth=hit.z-low))
        write_json(out/'root-contact-survey.json',dict(status='geometric base-center survey; needs visual contact review',buried_base_centers=contacts,note='Lowest wood vertices estimate each root base. Canopy and branches are excluded. This does not change approved geometry.'))
        owner,names=raster(receivers)
        known=np.asarray(Image.open(DEST/'bank-source-domain.png').convert('L'))>0
        bank_labels=[i for i,name in enumerate(names) if name!='ground']
        front=np.isin(owner,bank_labels)
        expected_missing=known & ~front
        first=known & front
        Image.fromarray(first.astype('uint8')*255).save(out/'bank-first-hit-domain.png')
        Image.fromarray(expected_missing.astype('uint8')*255).save(out/'bank-missing-domain.png')
        np.savez_compressed(out/'bank-ground-first-hit.npz',owner=owner,names=names)
        image=np.asarray(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB')).copy()
        image[expected_missing]=[255,0,0];Image.fromarray(image).save(out/'source-coverage-difference.png')
        bpy.ops.wm.save_as_mainfile(filepath=str(out/'scene.blend'))
        views=[]
        for i,angle in enumerate([0,math.pi/4,math.pi,5*math.pi/4]):
            direction=Vector((math.sin(angle)*math.cos(math.radians(35)),-math.cos(angle)*math.cos(math.radians(35)),math.sin(math.radians(35))))
            camera(scene,Vector((780,-440/SIN,50)),direction,1200,800,2100)
            scene.render.filepath=str(out/f'context-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name);views.append(out/f'context-{i}.png')
        sheet=Image.new('RGB',(1200,800))
        for i,path in enumerate(views):sheet.paste(Image.open(path).convert('RGB').resize((600,400)),(i%2*600,i//2*400))
        sheet.save(out/'context-sheet.png')
        source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB')
        rendered=Image.open(out/'source-camera.png').convert('RGBA');background=Image.new('RGBA',rendered.size,'#555');background.alpha_composite(rendered)
        comparison=Image.new('RGB',(1792,1152));comparison.paste(source.resize((896,576)),(0,0));comparison.paste(background.convert('RGB').resize((896,576)),(896,0));comparison.paste(Image.open(out/'source-coverage-difference.png').resize((896,576)),(0,576));comparison.paste(Image.open(out/'context-0.png').convert('RGB').resize((896,576)),(896,576));comparison.save(out/'comparison.png')
        if sha(WORKER/'model.blend')!=model_hash:raise ValueError('Bank model changed during review')
        write_json(out/'evidence.json',dict(status='private bank/ground receiver test; full-scene receiver ownership not certified',bank_model_sha256=sha(WORKER/'model.blend'),input_assembly_sha256=sha(OUT/'integration-review/scene.blend'),scene_sha256=sha(out/'scene.blend'),known_bank_pixels=int(known.sum()),first_hit_bank_pixels=int(first.sum()),missing_known_pixels=int(expected_missing.sum()),raster_scope='Bank0–4 versus ground only. Foreground native masks exclude scenery from known domain; actual foliage first-hit remains a separate joint check.',evidence={p.name:sha(p) for p in out.glob('*.png')},limitations=['Assembly inherits unfinished foliage layering, terrain texture and missing vegetation.','Native plateau changes source-depth ordering; root contacts require inspection.','Known source pixels that miss candidate geometry remain red and are not removed from the audit.']))
    finally:release()


if __name__=='__main__':main()
