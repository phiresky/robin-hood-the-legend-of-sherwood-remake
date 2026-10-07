"""Prepare/run a bounded western-shelf revision; --describe needs no Blender."""
import argparse
import json
import math
import sys
from collections import Counter
from pathlib import Path

R=Path(__file__).resolve().parents[3]
B=R/'level-editor/work/croisement03-refinement'
RECIPE=B/'restart2/bank-morphology-trace-v1/recipe.json'
OUT=B/'restart2/bank-shelf-prototype-v3'
S=math.sin(math.radians(35));C=math.cos(math.radians(35))


def shelf_mesh(shelf):
    # Paired observed front rings close onto a deep rear body and recessed foot.
    # Only these short front bands use source crease constraints. The body is inferred.
    vertices=[]
    for ring in shelf['proposed_short_face_rings']:
        upper=ring['upper_world'];lower=ring['lower_world'];x=upper[0]
        map_y=-upper[1]*S;top=upper[2]*C
        # Authored ramp53 plane in native coordinates, solved to map coordinates.
        def core_height(my):
            return max(1.,(246.60258125533795+.006610367178830212*x-.6040677165791918*my)/(1-.6040677165791918)-16)
        back_y=map_y-42
        back_top=top+2
        back_bottom=max(.2,min(back_top-10,core_height(back_y)-2))
        inset_y=map_y-5
        inset_bottom=max(.2,min(lower[2]*C-2,core_height(inset_y)-2))
        vertices.extend([upper,[x,-back_y/S,back_top/C],
            [x,-back_y/S,back_bottom/C],[x,-inset_y/S,inset_bottom/C],lower])
    rings=len(vertices)//5;faces=[]
    for i in range(rings-1):
        for j in range(5):faces.append([i*5+j,(i+1)*5+j,(i+1)*5+(j+1)%5,i*5+(j+1)%5])
    faces.extend([list(reversed(range(5))),list(range(len(vertices)-5,len(vertices)))])
    edges=Counter(tuple(sorted((a,b))) for f in faces for a,b in zip(f,f[1:]+f[:1]))
    assert set(edges.values())=={2}
    return dict(vertices=vertices,faces=faces,closed_edge_count=len(edges))


def description():
    recipe=json.loads(RECIPE.read_text())
    return dict(status='CPU builder prepared; Blender execution requires coordinator lane',
        source_recipe=str(RECIPE),output=str(OUT),
        geometry_scope='Replace visual ramp53 wedge roof with three source-traced shelf bands unioned onto a lowered solid core. Retain bank52 fixed crest and ramp54 unchanged for this incremental round.',
        retained_work='Main-bank52 white crease traces stay queued for the subsequent shoulder/front revision; this builder does not claim complete bank morphology.',
        bodies={s['id']:shelf_mesh(s) for s in recipe['shelves']},
        guards=['Boolean union must yield closed manifold geometry without zero-area faces.',
            'Preserve meaningful core volume; no disconnected strips or image-column extrusion.',
            'Reapply all3446 proposed seed pixels using per-face first-hit ownership.',
            'Run saved topology/context audit and Tree02–07 source guards, inspect native camera first plus all8 oblique and west-contact views.',
            'Classify575 mixed Tree01 rock rays separately; do not preserve false coarse bark projection.'],
        budget=dict(max_model_bytes=8*1024**2,max_round_bytes=32*1024**2,threads=2,min_free_bytes=10*1024**3),
        authorization='No approval, texture request or canonical mutation is implied.')


def run():
    import bpy,bmesh
    sys.path.insert(0,str(Path(__file__).parent))
    import restart2_bank_full_v1 as base
    original=base.mesh_object;plan=description();base.O=OUT
    def check(obj):
        bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(obj.data)
        bad=sum(not e.is_manifold for e in bm.edges);zero=sum(f.calc_area()<1e-8 for f in bm.faces);volume=abs(bm.calc_volume(signed=True))
        remaining=set(bm.verts);components=0
        while remaining:
            components+=1;pending=[remaining.pop()]
            while pending:
                vertex=pending.pop()
                for edge in vertex.link_edges:
                    other=edge.other_vert(vertex)
                    if other in remaining:remaining.remove(other);pending.append(other)
        bm.free()
        assert bad==0 and zero==0 and volume>1 and components==1,(obj.name,bad,zero,volume,components)
        return volume
    def build(name,points,scene):
        if name!='Candidate bank 53':return original(name,points,scene)
        core=[{**p,'z_top':max(1,p['z_top']-16)} for p in points]
        obj=original(name,core,scene);core_volume=check(obj)
        for label,body in plan['bodies'].items():
            mesh=bpy.data.meshes.new(label);mesh.from_pydata(body['vertices'],[],body['faces']);mesh.update();slab=bpy.data.objects.new(label,mesh);scene.collection.objects.link(slab);check(slab)
            bpy.context.view_layer.objects.active=obj;obj.select_set(True)
            modifier=obj.modifiers.new('Closed shelf union '+label,'BOOLEAN');modifier.operation='UNION';modifier.solver='EXACT';modifier.object=slab
            bpy.ops.object.modifier_apply(modifier=modifier.name);bpy.data.objects.remove(slab,do_unlink=True)
            assert check(obj)>=core_volume-.001,'Union lost core bulk'
        obj['morphology']='Three short source-traced shelf faces backed by a closed inferred solid core; full-bank morphology remains unfinished.'
        return obj
    base.mesh_object=build
    base.main()
    (OUT/'morphology-plan.json').write_text(json.dumps(plan,indent=2)+'\n')
    assert sum(p.stat().st_size for p in OUT.rglob('*') if p.is_file())<32*1024**2


if __name__=='__main__':
    args=sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else sys.argv[1:]
    parser=argparse.ArgumentParser();parser.add_argument('--describe',action='store_true');options=parser.parse_args(args)
    if options.describe:print(json.dumps(description(),indent=2))
    else:run()
