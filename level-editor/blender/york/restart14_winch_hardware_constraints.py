"""Measure room contact and attachment space before inventing hidden winch hardware."""
import ast,hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'winch-guided-entry-candidate-v1';OUT=WORK/'winch-guided-hardware-constraints-v1.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));up=Vector((0,0,1));screen_side=Vector((1,0,0));spacing=3.5/c
recipe=Path(__file__).with_name('restart14_winch_guided_entry.py');exec(compile(ast.Module(body=[n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(recipe),'exec'))
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.frame_set(88);bpy.context.view_layer.update();chain=[o for o in scene.objects if o.name.startswith('Guided chain link')];context=[o for o in scene.objects if o.type=='MESH' and o.get('native_patch')!='patch-004' and not o.hide_render];ctree,ct,cnames=tree(chain);btree,bt,bnames=tree(context);pairs=ctree.overlap(btree);counts={};bounds={}
if pairs:
    ai,bi=np.asarray(pairs).T;mask=crosses(ct[ai],bt[bi])|crosses(bt[bi],ct[ai])
    for ca,ba in zip(ai[mask],bi[mask]):
        name=bnames[ba];counts[name]=counts.get(name,0)+1;bounds.setdefault(name,[]).extend(ct[ca].tolist())
hits=[];allv=np.concatenate([np.asarray([o.matrix_world@v.co for v in o.data.vertices]) for o in chain]);top=allv[:,2].max();center=Vector(((allv[:,0].min()+allv[:,0].max())/2,(allv[:,1].min()+allv[:,1].max())/2,top-10))
for name,direction in [('up',(0,0,1)),('down',(0,0,-1)),('west',(-1,0,0)),('east',(1,0,0)),('north',(0,1,0)),('south',(0,-1,0))]:
    hit=btree.ray_cast(center,Vector(direction));hits.append({'direction':name,'origin_world':list(center),'owner':bnames[hit[2]] if hit[2] is not None else None,'point_world':list(hit[0]) if hit[0] is not None else None,'distance':hit[3]})
def bbox(points):return [[float(min(p[i] for p in points)),float(max(p[i] for p in points))] for i in range(3)]
traveller=[]
for frame in (0,22,36,44):
    scene.frame_set(frame*2);bpy.context.view_layer.update();o=scene.objects['Travelling round part solid drum'];points=[o.matrix_world@v.co for v in o.data.vertices];traveller.append({'frame':frame,'center_world':list(o.matrix_world.translation),'bbox_world':bbox(points),'center_game':[o.matrix_world.translation.x,-o.matrix_world.translation.y*s,o.matrix_world.translation.z*c]})
result={'status':'Private hardware constraints; no hardware authored or approved','model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'chain_context_exact_crossing_pairs':counts,'crossing_chain_triangle_bbox_world':{n:bbox(p) for n,p in bounds.items()},'chain_bbox_world':bbox(allv),'upper_probe':hits,'traveller':traveller,'limitations':['Existing room is a private cavity hypothesis, not approved final architecture.','Intersection bounds describe crossing triangles, not precise intersection curves.','Native periodic chain phase does not uniquely identify links; attachment kinematics remain uncertain.']};OUT.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
