"""Measure boundary foliage visibility against the exact joint neighbours."""
import json
import argparse
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release
from stage_review_scene import signature
from refinement_review import _tree


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--budget',type=int,default=256);parser.add_argument('--revision',default='v2');parser.add_argument('--rays-only',action='store_true');parser.add_argument('--candidate',type=Path)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    joint=OUT/'leaf-clump-joint-review/restart2-shrub93-boundary-v1'
    evidence=json.loads((joint/'evidence.json').read_text())
    if args.candidate:
        candidate=args.candidate.resolve();proof=json.loads((candidate/'preservation.json').read_text())
        digest=sha(candidate/'model.blend');assert digest==proof['model_sha256']
        assert sha(Path(proof['original_worker'])/'model.blend')==proof['original_model_sha256']
        evidence['inputs'][0]['workspace']=str(candidate);evidence['inputs'][0]['model_sha256']=digest
        evidence['workers'][0]=dict(path=str(candidate),model_sha256=digest)
    output=OUT/f'restart2-vegetation/shrub93-first-hit-{args.revision}';output.mkdir(exist_ok=False)
    bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene
    foliage=[]
    for row in evidence['inputs']:
        model=Path(row['workspace'])/'model.blend';assert sha(model)==row['model_sha256']
        with bpy.data.libraries.load(str(model),link=False) as (_,dest):
            dest.objects=[r['name'] for r in row['objects']]
        for obj in dest.objects:
            assert obj is not None
            scene.collection.objects.link(obj)
            parent=obj.parent
            while parent:
                if not parent.users_collection:scene.collection.objects.link(parent)
                parent=parent.parent
        bpy.context.view_layer.update()
        for obj in dest.objects:
            before=signature(obj);matrix=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=matrix
            assert signature(obj)==before;obj.hide_render=False
            if obj.get('asset_group')=='croisement02-shrub-93':foliage.append(obj)
    assert len(foliage)==4
    if args.rays_only:
        objects=[o for o in scene.objects if o.type=='MESH' and not o.hide_render]
        tree,owners,_=_tree(objects)
        boundary=np.asarray(Image.open(OUT/'mixed-wood-audit/boundary-roles76-93-v1/93-foliage93.png').convert('L'))>0
        rows=[]
        for y,x in zip(*np.nonzero(boundary)):
            origin=Vector((float(x)+.5,-(float(y)+.5)/SIN,0))+RAY*5000
            hit,normal,index,distance=tree.ray_cast(origin,-RAY)
            owner=owners[index] if hit is not None else None
            trace=[]
            if owner is None and hasattr(tree,'tree'):
                from physical_opacity import _alpha
                start=origin.copy();previous=None;step=.001
                for _ in range(200):
                    point,norm,idx,_=tree.tree.ray_cast(start,-RAY)
                    if point is None:break
                    rec=tree.records[idx]
                    trace.append(dict(object=owners[idx].name,point=list(point),alpha=_alpha(rec,point) if rec else 1.,normal_dot=norm.dot(-RAY),step=step))
                    step=step*2 if idx==previous else .001;previous=idx;start=point-RAY*step
            rows.append(dict(pixel=[int(x),int(y)],object=owner.name if owner else None,
                             asset=owner.get('asset_group') if owner else None,hit=list(hit) if hit else None,miss_trace=trace))
        write_json(output/'pixel-center-rays.json',dict(joint_evidence_sha256=sha(joint/'evidence.json'),workers=evidence['workers'],records=rows,
                    method='Shared physical-alpha BVH, exact source pixel centers and one-sided material rules; no image sampling budget.'))
        print([(r['pixel'],r['asset'])for r in rows]);return
    for obj in foliage:
        for i,material in enumerate(list(obj.data.materials)):
            material=material.copy();obj.data.materials[i]=material
            for node in material.node_tree.nodes:
                if node.type!='BSDF_PRINCIPLED':continue
                for name in ['Base Color','Emission Color']:
                    for link in list(node.inputs[name].links):material.node_tree.links.remove(link)
                    node.inputs[name].default_value=(1,0,1,1)
                node.inputs['Emission Strength'].default_value=1
    scene.render.engine='CYCLES';scene.cycles.samples=32;scene.cycles.transparent_max_bounces=args.budget
    scene.cycles.use_denoising=False
    scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA'
    world=bpy.data.worlds.new('Neutral');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.8;scene.world=world
    left,top,right,bottom=1320,707,1540,879
    camera_data=bpy.data.cameras.new('Exact source');camera_data.type='ORTHO';camera_data.ortho_scale=right-left;camera_data.clip_end=10000
    camera=bpy.data.objects.new(camera_data.name,camera_data);scene.collection.objects.link(camera);scene.camera=camera
    center=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));camera.location=center+RAY*5000
    camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
    scene.render.resolution_x=(right-left)*3;scene.render.resolution_y=(bottom-top)*3;scene.render.resolution_percentage=100
    scene.render.filepath=str(output/'joint-labels.png');bpy.ops.render.render(write_still=True)
    rgba=np.asarray(Image.open(output/'joint-labels.png').convert('RGBA'))[1::3,1::3]
    own=(rgba[:,:,0]>240)&(rgba[:,:,1]<40)&(rgba[:,:,2]>240)&(rgba[:,:,3]>127)
    for obj in scene.objects:
        if obj.type=='MESH':obj.hide_render=obj not in foliage
    scene.render.filepath=str(output/'isolated-labels.png');bpy.ops.render.render(write_still=True)
    isolated=np.asarray(Image.open(output/'isolated-labels.png').convert('RGBA'))[1::3,1::3]
    isolated_own=(isolated[:,:,0]>240)&(isolated[:,:,1]<40)&(isolated[:,:,2]>240)&(isolated[:,:,3]>127)
    records=[]
    for role,path in [('observed503',OUT/'mixed-wood-audit/domain-503.png'),('inferred6003',OUT/'mixed-wood-audit/boundary-roles76-93-v1/93-foliage93.png')]:
        mask=np.asarray(Image.open(path).convert('L'))[top:bottom,left:right]>0
        ys,xs=np.nonzero(mask&~own)
        records.append(dict(role=role,mask_sha256=sha(path),pixels=int(mask.sum()),own_first_hit=int((mask&own).sum()),isolated_own=int((mask&isolated_own).sum()),
                            missing_rgba=[dict(pixel=[int(x+left),int(y+top)],joint=rgba[y,x].tolist(),isolated=isolated[y,x].tolist())for y,x in zip(ys,xs)],
                            missing_pixels=[[int(x+left),int(y+top)]for y,x in zip(ys,xs)]))
    for row in evidence['inputs']:assert sha(Path(row['workspace'])/'model.blend')==row['model_sha256']
    write_json(output/'evidence.json',dict(status='Measured actual source-ray visibility; no saved model changes',
               joint_evidence_sha256=sha(joint/'evidence.json'),workers=evidence['workers'],records=records,transparent_bounces=args.budget,
               image_sha256=sha(output/'joint-labels.png'),source_crop=[left,top,right,bottom],
               limitations=['Magenta alters RGB only; texture alpha and one-sided material chains retained.',
                   'Prior canonical source and user approval remain unchanged.']))
    print(records)


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
