"""Audit the four private hall states without changing any candidate."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement/restart2'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,bmesh
states=[('initial-initial',OUT/'hall-states-grounded-v1/initial-initial'),
        ('initial-applied',OUT/'hall-states-grounded-v3/initial-applied'),
        ('applied-initial',OUT/'hall-states-grounded-v3/applied-initial'),
        ('applied-applied',OUT/'hall-grounded-v1')]
records=[];invariants=[]
for name,workspace in states:
    model=workspace/'model.blend';bpy.ops.wm.open_mainfile(filepath=str(model))
    bpy.context.window.scene=bpy.data.scenes['york Refinement'];bpy.context.view_layer.update()
    objects=[o for o in bpy.data.collections['york Working'].all_objects
             if o.type=='MESH' and not o.hide_render and o.get('asset_group')=='york-castle-great-hall']
    signatures={};parts=[]
    for obj in objects:
        bm=bmesh.new();bm.from_mesh(obj.data)
        nonmanifold=sum(not e.is_manifold for e in bm.edges);volume=bm.calc_volume();bm.free()
        if nonmanifold or abs(volume)<1e-6:raise ValueError(f'Non-solid hall component {name}: {obj.name}')
        signature={'points':[list(obj.matrix_world@v.co) for v in obj.data.vertices],
                   'faces':[list(p.vertices)for p in obj.data.polygons],
                   'source_node':obj.get('source_node'),'projection_component':obj.get('projection_component')}
        digest=hashlib.sha256(json.dumps(signature,sort_keys=True).encode()).hexdigest()
        parts.append({'name':obj.name,'source_node':obj.get('source_node'),'projection_component':obj.get('projection_component'),
                      'world_geometry_sha256':digest,'nonmanifold_edges':nonmanifold,'signed_volume':volume})
        if obj.get('source_node') not in ('building-799','scenery-york-great-hall-upper-front-wall'):signatures[obj.name]=digest
    invariants.append(signatures)
    records.append({'state':name,'model_sha256':hashlib.sha256(model.read_bytes()).hexdigest(),'parts':parts})
if any(r!=invariants[0]for r in invariants[1:]):raise ValueError('A non-roof/non-cover hall component changed between states')
result={'status':'PASS','scope':'All visible hall components closed; every non-roof/non-cover world-space face identical across four states.',
        'invariant_parts':len(invariants[0]),'states':records,
        'foundation_support_evidence':str(OUT/'hall-grounded-v1/terrain-contact-v1/evidence.json'),
        'approval':'Technical state audit only; no user geometry approval.'}
p=OUT/'hall-four-state-review-v1/state-invariants.json'
if p.exists():raise FileExistsError(p)
p.write_text(json.dumps(result,indent=2)+'\n');print(p)
