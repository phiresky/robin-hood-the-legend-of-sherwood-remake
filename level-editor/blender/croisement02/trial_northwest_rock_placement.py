"""Private source-ray placement diagnostic for northwest rock/bank depth conflict."""
import argparse,json,math
from pathlib import Path
import sys
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector,Matrix
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from correct_bank_foot import surface,digest
from workspace_components import appearance_state
from refinement_workspace import _geometry
from review_bank_candidate import camera


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--part',choices=['all','building-035'],default='all');parser.add_argument('--output',required=True);parser.add_argument('--shift',type=float,default=51.5);parser.add_argument('--ramp-x',type=float,nargs=2);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    output=OUT/args.output
    if output.exists():raise FileExistsError(output)
    output.mkdir()
    bank=OUT/'restart2-bank321/foot-candidate-v2/worker.blend';source=OUT/'restart2-northwest-rock/experiment-sloping-cap-v1/bake-single-v2/worker.blend'
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update()
        bank_objs=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank']
        bank_tree=surface(bank_objs);bank_matrices={o.name:[list(r) for r in o.matrix_world] for o in bank_objs}
        bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.preferences.filepaths.save_version=0;bpy.context.view_layer.update()
        scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene
        rock=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-northwest-rock-outcrop']
        before_tree=surface(rock);world_before={o.name:np.array([o.matrix_world@v.co for v in o.data.vertices]) for o in rock}
        local={o.name:[list(v.co) for v in o.data.vertices] for o in rock};appearance={o.name:digest(appearance_state(o,{})) for o in rock};foreign={o.name:digest(_geometry(o)) for o in scene.objects if o not in rock}
        shift=args.shift
        for obj in rock:
            if args.part != 'all' and obj.get('source_node') != args.part:continue
            if args.ramp_x:
                low,high=args.ramp_x
                if high<=low:raise ValueError('Ramp interval must increase')
                inverse=obj.matrix_world.inverted()
                for vertex in obj.data.vertices:
                    world=obj.matrix_world@vertex.co;weight=max(0.,min(1.,(world.x-low)/(high-low)));vertex.co=inverse@(world+Vector(RAY)*(shift*weight))
            else:
                matrix=obj.matrix_world.copy();matrix.translation+=Vector(RAY)*shift;obj.matrix_world=matrix
        bpy.context.view_layer.update();after_tree=surface(rock)
        projection_errors=[]
        for obj in rock:
            if (not args.ramp_x and local[obj.name]!=[list(v.co) for v in obj.data.vertices]) or appearance[obj.name]!=digest(appearance_state(obj,{})):raise ValueError('Protected geometry/UV/texture changed')
            a=world_before[obj.name];b=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);projection=lambda p:np.column_stack((p[:,0],-p[:,1]*SIN-p[:,2]*COS));projection_errors.append(float(abs(projection(a)-projection(b)).max()))
        if max(projection_errors)>.001:raise ValueError('Source silhouette shifted')
        if foreign!={o.name:digest(_geometry(o)) for o in scene.objects if o not in rock}:raise ValueError('Unrelated geometry changed')
        domain=np.array(Image.open(OUT/'northwest-rock-source-revision/domain-380.png').convert('L'))>0;bank_domain=np.array(Image.open(OUT/'terrain-bank-candidate/bank-source-domain.png').convert('L'))>0
        before_visible=np.zeros_like(domain);after_visible=np.zeros_like(domain);extent=np.vstack(list(world_before.values()));max_y=int(np.ceil((-extent[:,1]*SIN-extent[:,2]*COS).max()))+2;max_x=int(np.ceil(extent[:,0].max()))+2
        for y in range(max_y):
            for x in range(max_x):
                origin=Vector((x+.5,-(y+.5)/SIN,0))+Vector(RAY)*10000
                b,_,_,bd=bank_tree.ray_cast(origin,-Vector(RAY));a,_,_,ad=before_tree.ray_cast(origin,-Vector(RAY));c,_,_,cd=after_tree.ray_cast(origin,-Vector(RAY))
                before_visible[y,x]=a is not None and(b is None or ad<bd)
                after_visible[y,x]=c is not None and(b is None or cd<bd)
        gained=after_visible&~before_visible
        stats=dict(new_visible_pixels=int(gained.sum()),new_visible_native_rock=int((gained&domain).sum()),new_visible_native_bank=int((gained&bank_domain).sum()),new_visible_other=int((gained&~domain&~bank_domain).sum()),native_rock_visible_before=int((before_visible&domain).sum()),native_rock_visible_after=int((after_visible&domain).sum()),native_rock_domain_pixels=int(domain.sum()),native_rock_still_missing=int((domain&~after_visible).sum()))
        for name,data in [('gained',gained),('before-visible',before_visible),('after-visible',after_visible)]:Image.fromarray(data.astype('uint8')*255).save(output/(name+'.png'))
        bpy.ops.wm.save_as_mainfile(filepath=str(output/'worker.blend'))
        write_json(output/'validation.json',dict(status='Private placement diagnostic; root/user review pending',model_sha256=sha(output/'worker.blend'),source_model_sha256=sha(source),bank_model_sha256=sha(bank),shifted_part=args.part,source_ray_shift=shift,world_shift=list(Vector(RAY)*shift),max_source_projection_error=max(projection_errors),local_geometry_exact=not bool(args.ramp_x),uv_images_exact=True,ramp_x=args.ramp_x,unrelated_geometry_unchanged=True,coverage=stats,user_approval=None,prior_texture_approval_inherited=False))
        # Import reviewed bank matrices from the opened source, never from an
        # unlinked library object whose matrix may still be identity.
        bank_names=list(bank_matrices)
        with bpy.data.libraries.load(str(bank),link=False) as (available,loaded):loaded.objects=bank_names.copy()
        for name,obj in zip(bank_names,loaded.objects):scene.collection.objects.link(obj);obj.parent=None;obj.matrix_world=Matrix(bank_matrices[name])
        bpy.context.view_layer.update()
        for obj in scene.objects:
            if obj.type=='MESH':obj.hide_render=obj not in rock+loaded.objects
        camera(scene,Vector((896,-576/SIN,0)),Vector(RAY),1792,1152,1792);scene.render.filepath=str(output/'native-contact.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        for i,angle in enumerate([math.pi/4,-math.pi/4]):
            direction=Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));camera(scene,Vector((35,-230,55)),direction,640,480,360);scene.render.filepath=str(output/f'oblique-contact-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
        print(json.dumps(stats))
    finally:release()


if __name__=='__main__':main()
