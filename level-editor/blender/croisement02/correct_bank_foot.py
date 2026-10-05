"""Private minimal bank-foot displacement, retaining exact atlas pixels and UVs."""
import hashlib
import json
from pathlib import Path
import sys
import bpy
import numpy as np
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
from workspace_components import appearance_state
from refinement_workspace import _geometry
from review_bank_candidate import camera


def digest(value):
    return hashlib.sha256(json.dumps(value,sort_keys=True).encode()).hexdigest()


def surface(objects):
    vertices=[];triangles=[]
    for obj in objects:
        start=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices)
        obj.data.calc_loop_triangles();triangles.extend(tuple(start+i for i in t.vertices) for t in obj.data.loop_triangles)
    return BVHTree.FromPolygons(vertices,triangles,all_triangles=True)


def hits(tree,coords):
    ray=Vector(RAY);result=[]
    for y,x in coords:
        origin=Vector((x+.5,-(y+.5)/SIN,0))+ray*10000
        p,_,_,distance=tree.ray_cast(origin,-ray)
        result.append(p is not None and distance<=origin.z/ray.z+.001)
    return np.array(result)


def main():
    output=OUT/'restart2-bank321/foot-candidate-v2'
    if output.exists():raise FileExistsError(output)
    output.mkdir(parents=True)
    experiment=OUT/'texture-fill-round-2/croisement02-north-woodland-bank/complete-preparation/experiment-ground-retry-v2'
    source=experiment/'bake-v1/worker.blend'
    classification=OUT/'restart2-bank321/classification-v2/classification.json'
    report=json.loads(classification.read_text());source_hash=sha(source)
    if source_hash!=report['model_sha256']:raise ValueError('Classified model changed')
    targets=[r for r in report['rows'] if r['category']=='beyond-one-pixel-needs-source-review']
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.preferences.filepaths.save_version=0
        manifest=json.loads((experiment/'views.json').read_text());scene=bpy.data.scenes[manifest['scene_name']];bpy.context.window.scene=scene
        objects=[scene.objects[n] for n in manifest['object_names']]
        original={o.name:np.array([v.co[:] for v in o.data.vertices]) for o in objects}
        appearance={o.name:digest(appearance_state(o,{})) for o in objects}
        foreign={o.name:digest(_geometry(o)) for o in scene.objects if o not in objects}
        known=np.asarray(Image.open(OUT/'terrain-bank-candidate/bank-source-domain.png').convert('L'))>0
        coords=np.argwhere(known);before=hits(surface(objects),coords)
        # Pick the local southern boundary at each target's X. Only low toe
        # edges belonging to the two traced bank pieces may move.
        changed=set();steps=[]
        for r in targets:
            x,y=r['x']+.5,r['y']+.5
            if hits(surface(objects),[(r['y'],r['x'])])[0]:continue
            options=[]
            for obj in objects:
                if obj.get('source_node') not in {'building-000','building-003'}:continue
                world=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);projected=np.column_stack((world[:,0],-world[:,1]*SIN-world[:,2]*COS))
                for edge in obj.data.edges:
                    a,b=edge.vertices
                    if max(world[a,2],world[b,2])>1 or abs(projected[b,0]-projected[a,0])<1e-5:continue
                    t=(x-projected[a,0])/(projected[b,0]-projected[a,0])
                    if not 0<=t<=1:continue
                    ey=projected[a,1]*(1-t)+projected[b,1]*t
                    gap=y-ey
                    if -.1<=gap<=6:options.append((gap,obj.name,int(a),int(b),float(ey)))
            if not options:raise ValueError(f'No bounded toe edge at {x},{y}')
            gap,name,a,b,ey=min(options)
            obj=scene.objects[name];delta=gap+.16
            # Move both endpoint positions, including duplicate lower skirt
            # vertices at the same XY, so the shallow contact wall stays closed.
            world=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);selected=[]
            for index,p in enumerate(world):
                if p[2]<=1 and any(np.linalg.norm(p[:2]-world[k,:2])<.001 for k in [a,b]):selected.append(index)
            inv=obj.matrix_world.inverted()
            for index in selected:
                p=Vector(world[index]);p.y-=delta/SIN;obj.data.vertices[index].co=inv@p;changed.add((name,index))
            obj.data.update();steps.append(dict(target=[r['x'],r['y']],object=name,vertices=selected,source_y_displacement=float(delta)))
        after=hits(surface(objects),coords)
        if (before&~after).any():raise ValueError('Known bank center coverage regressed')
        if not hits(surface(objects),[(r['y'],r['x']) for r in targets]).all():raise ValueError('True gaps remain')
        movements=[]
        for obj in objects:
            now=np.array([v.co[:] for v in obj.data.vertices]);indices=np.flatnonzero(np.max(abs(now-original[obj.name]),axis=1)>1e-7)
            for i in indices:
                old=obj.matrix_world@Vector(original[obj.name][i]);new=obj.matrix_world@obj.data.vertices[i].co
                if abs(new.x-old.x)>.001 or abs(new.z-old.z)>.001:raise ValueError('Toe correction changed height or X')
                if abs(new.y-old.y)*SIN>6:raise ValueError('Toe correction exceeds6 source pixels')
                movements.append(dict(object=obj.name,vertex=int(i),before=list(old),after=list(new),source_y_displacement=-(new.y-old.y)*SIN))
            if digest(appearance_state(obj,{}))!=appearance[obj.name]:raise ValueError('Atlas, UV or material changed')
        if {o.name:digest(_geometry(o)) for o in scene.objects if o not in objects}!=foreign:raise ValueError('Unrelated geometry changed')
        bpy.ops.wm.save_as_mainfile(filepath=str(output/'worker.blend'))
        saved=sha(output/'worker.blend')
        write_json(output/'validation.json',dict(status='PASS private geometry candidate',source_model_sha256=source_hash,model_sha256=saved,classification_sha256=sha(classification),known_pixels=int(known.sum()),center_hits_before=int(before.sum()),center_hits_after=int(after.sum()),coverage_regressions=0,corrected_true_gaps=len(targets),moved_vertices=movements,steps=steps,exact_uv_material_image=True,unrelated_geometry_unchanged=True,user_approval=None,prior_texture_approval_inherited=False))
        missing=np.zeros_like(known);missing[coords[:,0],coords[:,1]]=~after
        Image.fromarray(missing.astype('uint8')*255).save(output/'remaining-missing.png')
        render(experiment/'views.json',output/'actual',modes=('solid','textured'),width=384)
        # Original map camera for native-boundary inspection. All non-bank
        # objects are hidden; the saved worker is not changed by these views.
        for obj in scene.objects:
            if obj.type=='MESH':obj.hide_render=obj not in objects
        camera(scene,Vector((896,-576/SIN,0)),Vector(RAY),1792,1152,1792)
        scene.render.filepath=str(output/'native-camera.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        if sha(source)!=source_hash or sha(output/'worker.blend')!=saved:raise ValueError('Frozen model changed')
        print(json.dumps({'model_sha256':saved,'moved_vertices':len(movements),'remaining_misses':int(missing.sum())}))
    finally:release()


if __name__=='__main__':main()
