"""Review visible root contacts without changing approved tree placement."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from prepare_bank_candidate import DEST,WORKER,ASSET,OUT,SIN,COS
from tree_geometry import RAY
from review_bank_candidate import camera
from evidence_io import sha,write_json
from render_slots import acquire,release


def bvh(objects):
    vertices=[];triangles=[]
    for obj in objects:
        offset=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices)
        obj.data.calc_loop_triangles();triangles.extend(tuple(offset+i for i in t.vertices) for t in obj.data.loop_triangles)
    return BVHTree.FromPolygons(vertices,triangles,all_triangles=True)


def solid(name,color):
    mat=bpy.data.materials.new(name);mat.diffuse_color=(*color,1);mat.use_nodes=True
    mat.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(*color,1)
    mat.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=1
    return mat


def main():
    out=DEST/'root-contacts';out.mkdir(exist_ok=True)
    source_scene=DEST/'integration/scene.blend';source_hash=sha(source_scene)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source_scene))
        collection=bpy.data.collections['Croisement02 Working']
        banks=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==ASSET]
        bank_bvh=bvh(banks)
        inventory=json.loads((OUT/'review-mask-inventory.json').read_text())['masks']
        bitmaps={}
        for row in inventory:
            if not row.get('png'):continue
            a=np.asarray(Image.open(row['png']).convert('L'))>0
            x,y=row['box_top_left'];h,w=a.shape;full=np.zeros((1152,1792),bool)
            full[max(0,y):min(1152,y+h),max(0,x):min(1792,x+w)] = a[max(0,-y):min(h,1152-y),max(0,-x):min(w,1792-x)]
            bitmaps[row['index']]=full
        foreground=np.logical_or.reduce([a for i,a in bitmaps.items() if 54<=i<=93 or 128<=i<=137])
        records=[]
        source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB')
        for index in ([] if '--ramp-only' in sys.argv else [3,14,21]):
            asset=f'croisement02-tree-{index:02}'
            trees=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==asset and o.get('projection_component')!='crown']
            wood_bvh=bvh(trees)
            points=[o.matrix_world@v.co for o in trees for v in o.data.vertices]
            low=min(p.z for p in points);base=[p for p in points if p.z<=low+.1];center=sum(base,Vector())/len(base)
            visible=bitmaps[index]&~foreground
            ys,xs=np.nonzero(visible);hidden=np.zeros_like(visible);miss=np.zeros_like(visible)
            for y,x in zip(ys,xs):
                start=Vector((float(x)+.5,-(float(y)+.5)/SIN,0))+RAY*5000
                wood_hit,_,_,wood_dist=wood_bvh.ray_cast(start,-RAY)
                bank_hit,_,_,bank_dist=bank_bvh.ray_cast(start,-RAY)
                if wood_hit is None:miss[y,x]=True
                elif bank_hit is not None and bank_dist<wood_dist-.05:hidden[y,x]=True
            scene=bpy.data.scenes.new('Root contact '+str(index));scene.world=bpy.data.worlds.new('Contact world');scene.world.color=(.35,.35,.35)
            bankmat=solid('Contact bank',(0.22,.34,.45));woodmat=solid('Contact wood',(.62,.25,.07))
            for original in banks+trees:
                obj=original.copy();obj.data=original.data.copy();obj.parent=None;obj.matrix_world=original.matrix_world.copy();obj.hide_render=False
                obj.data.materials.clear();obj.data.materials.append(bankmat if original in banks else woodmat)
                for face in obj.data.polygons:face.material_index=0
                scene.collection.objects.link(obj)
            light=bpy.data.lights.new('Contact light','AREA');light.energy=20000;light.shape='DISK';light.size=350
            lamp=bpy.data.objects.new(light.name,light);scene.collection.objects.link(lamp);lamp.location=center+Vector((-100,-200,350));lamp.rotation_euler=(center-lamp.location).to_track_quat('-Z','Y').to_euler()
            target=center.copy();target.z=50
            paths=[]
            for j,direction in enumerate([RAY,Vector((.65,-.55,.55)).normalized(),Vector((-.65,-.55,.55)).normalized()]):
                camera(scene,target,direction,480,480,200)
                scene.render.filepath=str(out/f'tree-{index:02}-contact-{j}.png');bpy.ops.render.render(write_still=True,scene=scene.name);paths.append(Path(scene.render.filepath))
            px=int(center.x);py=int(-center.y*SIN-center.z*COS)
            box=(max(0,px-100),max(0,py-100),min(1792,px+100),min(1152,py+100))
            diagnostic=np.asarray(source).copy();diagnostic[hidden]=[255,0,0];diagnostic[miss]=[0,255,255]
            Image.fromarray(diagnostic).crop(box).resize((480,480)).save(out/f'tree-{index:02}-source.png')
            sheet=Image.new('RGB',(960,960),'#333')
            sheet.paste(Image.open(out/f'tree-{index:02}-source.png'),(0,0))
            for j,path in enumerate(paths):sheet.paste(Image.open(path).convert('RGB'),(((j+1)%2)*480,((j+1)//2)*480))
            sheet.save(out/f'tree-{index:02}-sheet.png')
            records.append(dict(asset=asset,base_center=list(center),visible_native_wood_pixels=int(visible.sum()),native_wood_pixels_without_geometry=int(miss.sum()),native_wood_pixels_hidden_by_bank=int(hidden.sum()),source_crop=box,sheet_sha256=sha(out/f'tree-{index:02}-sheet.png')))
        if '--ramp-only' not in sys.argv:write_json(out/'evidence.json',dict(status='contact diagnostic; no approved placement changes',source_scene_sha256=source_hash,records=records,method='Native wood mask minus native canopy/shrub masks. Cast source rays against exact saved wood geometry and bank separately. Brown wood and blue bank use temporary diagnostic materials; source panel red=wood hidden by bank, cyan=no wood ray hit.',limitations=['Native foreground masks are a conservative visibility filter; missing authored foliage and runtime states remain separate.','Base-center burial alone is not a relocation criterion.','Diagnostic material overrides exist only in temporary render scenes.']))
        scene=bpy.data.scenes.new('Northeast ramp detail');scene.world=bpy.data.worlds.new('Ramp world');scene.world.color=(.35,.35,.35)
        for original in banks:
            obj=original.copy();obj.parent=None;obj.matrix_world=original.matrix_world.copy();obj.hide_render=False;scene.collection.objects.link(obj)
        target=Vector((1200,-235/SIN,14))
        light=bpy.data.lights.new('Ramp light','AREA');light.energy=30000;light.size=350
        lamp=bpy.data.objects.new(light.name,light);scene.collection.objects.link(lamp);lamp.location=target+Vector((-100,-200,350));lamp.rotation_euler=(target-lamp.location).to_track_quat('-Z','Y').to_euler()
        neutral=solid('Ramp solid',(.42,.42,.42));paths=[]
        for i,direction in enumerate([RAY,Vector((.7,-.6,.6)).normalized(),Vector((-.7,.6,.6)).normalized()]):
            camera(scene,target,direction,480,480,240)
            for mode in ['actual','solid']:
                scene.view_layers[0].material_override=neutral if mode=='solid' else None
                scene.render.filepath=str(out/f'ramp3-{i}-{mode}.png');bpy.ops.render.render(write_still=True,scene=scene.name);paths.append(Path(scene.render.filepath))
        sheet=Image.new('RGB',(960,1440))
        for i,path in enumerate(paths):sheet.paste(Image.open(path).convert('RGB'),(i%2*480,i//2*480))
        sheet.save(out/'ramp3-detail-sheet.png')
        write_json(out/'ramp3-evidence.json',dict(source_scene_sha256=source_hash,sheet_sha256=sha(out/'ramp3-detail-sheet.png'),views={p.name:sha(p) for p in paths}))
    finally:release()


if __name__=='__main__':main()
