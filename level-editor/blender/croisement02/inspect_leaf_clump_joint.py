"""Private leaf-clump neighbourhood views with unchanged selected scenery context."""
import argparse
import json
import math
import sys
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,scenery_workspace,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from stage_review_scene import signature


def worker(index):
    rounds={77:16,78:12,83:18,84:12,74:15,85:15,86:15,87:20,88:15,89:17,90:15}
    return OUT/f'understory-round-{rounds[index]}/assets/croisement02-shrub-{index}'


def run(label,indices,include_bank=None,context_workers=()):
    destination=OUT/'leaf-clump-joint-review'/label;destination.mkdir(parents=True,exist_ok=False)
    bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene
    collection=bpy.data.collections.new('Ground plant joint review');scene.collection.children.link(collection)
    records=[];plant_objects=[]
    inputs=[(worker(i),'private plant') for i in indices]+[(p,'selected neighbouring vegetation') for p in context_workers]
    if include_bank is None:include_bank=False
    if include_bank:inputs.append((scenery_workspace('croisement02-north-woodland-bank'),'selected bank context'))
    for workspace,role in inputs:
        model=workspace/'model.blend';digest=sha(model)
        audit=json.loads((workspace/'inspection/saved-model-audit.json').read_text())
        if audit['status']!='PASS' or audit['model_sha256']!=digest:raise ValueError('Stale worker audit')
        names=[r['object'] for r in audit['objects']]
        with bpy.data.libraries.load(str(model),link=False) as (src,dst):dst.objects=names
        for obj in dst.objects:
            collection.objects.link(obj)
            parent=obj.parent
            while parent:
                if not parent.users_collection:collection.objects.link(parent)
                parent=parent.parent
        bpy.context.view_layer.update()
        bindings=[]
        for obj in dst.objects:
            before=signature(obj);matrix=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False
            if signature(obj)!=before:raise ValueError('Placement changed during append')
            bindings.append(dict(name=obj.name,source_node=obj.get('source_node'),signature=before))
            if role=='private plant':plant_objects.append(obj)
        records.append(dict(workspace=str(workspace),model_sha256=digest,role=role,objects=bindings))
    points=[obj.matrix_world@v.co for obj in plant_objects for v in obj.data.vertices]
    center=Vector([(min(p[i] for p in points)+max(p[i] for p in points))/2 for i in range(3)])
    if not include_bank:
        bpy.ops.mesh.primitive_plane_add(size=500,location=(center.x,center.y,0))
        floor=bpy.context.object;floor.name='Neutral ground contact guide (not source artwork)'
        mat=bpy.data.materials.new('Neutral contact plane');mat.diffuse_color=(.15,.16,.12,1);floor.data.materials.append(mat)
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=64
    scene.render.resolution_x=640;scene.render.resolution_y=640;scene.render.resolution_percentage=100
    scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA'
    scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    world=bpy.data.worlds.new('Neutral');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.8;scene.world=world
    data=bpy.data.cameras.new('Fixed joint camera');data.type='ORTHO';data.clip_end=20000
    camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera
    directions=[('source',RAY)]+[(f'oblique-{i}',Vector((math.sin(i*math.tau/8)*math.cos(.5),-math.cos(i*math.tau/8)*math.cos(.5),math.sin(.5)))) for i in range(8)]
    views=[]
    for name,direction in directions:
        camera.location=center+direction*5000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();bpy.context.view_layer.update()
        local=[camera.matrix_world.inverted()@p for p in points]
        data.ortho_scale=max(max(p[i] for p in local)-min(p[i] for p in local) for i in (0,1))*1.3
        offset=Vector([(max(p[i] for p in local)+min(p[i] for p in local))/2 for i in (0,1)]+[0]);camera.location+=camera.matrix_world.to_quaternion()@offset
        scene.render.filepath=str(destination/f'{name}.png');bpy.ops.render.render(write_still=True)
        views.append(dict(image=name+'.png',sha256=sha(destination/f'{name}.png'),location=list(camera.location),rotation=list(camera.rotation_euler),ortho_scale=data.ortho_scale))
        if name=='source':
            visibility={o:o.hide_render for o in scene.objects if o.type=='MESH'}
            for o in visibility:o.hide_render=o not in plant_objects
            scene.render.filepath=str(destination/'source-plants.png');bpy.ops.render.render(write_still=True)
            for o,hidden in visibility.items():o.hide_render=hidden
            x,y,z=camera.location;half=data.ortho_scale/2;sy=-y*SIN-z*COS
            box=[x-half,sy-half,x+half,sy+half]
            original=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA')
            native=original.transform((640,640),Image.Transform.EXTENT,box,Image.Resampling.NEAREST)
            native.save(destination/'native-source.png')
            Image.alpha_composite(native,Image.open(destination/'source-plants.png').convert('RGBA')).save(destination/'source-overlay.png')
            rendered=Image.open(destination/'source-plants.png').convert('RGBA')
            scale=rendered.width/(box[2]-box[0])
            layer=rendered.transform(original.size,Image.Transform.AFFINE,(scale,0,-box[0]*scale,0,scale,-box[1]*scale),Image.Resampling.BICUBIC)
            native_scale=Image.alpha_composite(original,layer);native_scale.save(destination/'native-scale-full-context.png')
            native_crop=[max(0,math.floor(box[0])-30),max(0,math.floor(box[1])-30),min(original.width,math.ceil(box[2])+30),min(original.height,math.ceil(box[3])+30)]
            native_scale.crop(native_crop).save(destination/'native-scale-context.png')
            write_json(destination/'native-scale-evidence.json',dict(status='Display-only: one native map pixel per output pixel',
                crop_box=native_crop,source_render_sha256=sha(destination/'source-plants.png'),
                images={n:sha(destination/n) for n in ['native-scale-full-context.png','native-scale-context.png']},
                note='Resampled review display only; no texture generation or frozen technical input is resized.'))
            write_json(destination/'source-framing.json',dict(native_source_box=box,source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),
                images={n:sha(destination/n) for n in ['native-source.png','source-plants.png','source-overlay.png']},
                note='Native artwork with exact-camera plant overlay; floor/bank suppressed only for this source comparison.'))
    sheet=Image.new('RGB',(2880,350),'#444444');draw=ImageDraw.Draw(sheet)
    for i,v in enumerate(views):
        im=Image.open(destination/v['image']).convert('RGBA');im.thumbnail((320,320));sheet.paste(im,(i*320,25),im);draw.text((i*320+5,5),v['image'],fill='white')
    sheet.save(destination/'sheet.png')
    for r in records:
        if sha(Path(r['workspace'])/'model.blend')!=r['model_sha256']:raise ValueError('Worker changed during joint render')
    write_json(destination/'evidence.json',dict(status='private grouped placement; manual review pending',inputs=records,workers=[dict(path=r['workspace'],model_sha256=r['model_sha256']) for r in records],views=views,sheet_sha256=sha(destination/'sheet.png'),
        limitation='Contact review only. New plants remain unselected; wider neighboring vegetation/terrain ownership review is still required.'))

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('index',type=int,choices=[74,77,78,83,86,87,88,89,90]);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    contexts={74:['east-rail-fence'],77:['southwest-field-wattle-fence'],78:['southwest-log-pile'],83:['southwest-rock-outcrop','shrub-81'],86:['logging-clearing-log','logging-clearing-stumps','north-kindling-bundle'],87:['east-upright-rail-fence-94'],88:['woodcutters-shed'],89:['supplemental-wood-44'],90:['supplemental-wood-44']}
    neighbours=[scenery_workspace('croisement02-'+s) for s in contexts[args.index]]
    if args.index==87:neighbours += [tree_workspace(i) for i in (39,40)]
    if args.index==88:neighbours += [tree_workspace(i) for i in (39,40,47)]
    if args.index==83:neighbours += [tree_workspace(i) for i in (29,30)]
    if args.index in (89,90):neighbours += [tree_workspace(i) for i in (43,45,46)]
    target=worker(args.index);label=f'native-{args.index}-'+sha(target/'model.blend')[:8]+'-'+sha(neighbours[0]/'model.blend')[:8]
    acquire()
    try:run(label,[83,84] if args.index==83 else [args.index],False,neighbours)
    finally:release()
