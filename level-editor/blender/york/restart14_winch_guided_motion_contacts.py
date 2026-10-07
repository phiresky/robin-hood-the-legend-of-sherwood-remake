"""Check a fixed guided chain path against every measured crank/descent pose."""
import ast, hashlib, json, math, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'winch-components-source-v2';OUT=WORK/'winch-guided-entry-study-v3/motion-contacts.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1));screen_side=Vector((1,0,0));spacing=3.5/c
for name in ('restart13_winch_return_study.py','restart14_winch_guided_entry.py'):
    recipe=Path(__file__).with_name(name);exec(compile(ast.Module(body=[n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(recipe),'exec'))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0))
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scope=set(json.loads((BASE/'component-freeze.json').read_text())['scope']);objects=[o for o in scene.objects if o.name in scope]
pose,params,path=guided_route();rx=1.5;wire=.35;perimeter=2*math.pi*rx+4*(3-rx);local=[];faces=[]
for j in range(32):
    p,n=capsule(j*perimeter/32,rx)
    for k in range(8):phi=k*math.tau/8;local.append(p+n*(wire*math.cos(phi))+Vector((0,0,wire*math.sin(phi))))
for j in range(32):
    for k in range(8):
        a=j*8+k;b=((j+1)%32)*8+k;cc=((j+1)%32)*8+(k+1)%8;d=j*8+(k+1)%8;faces.extend(((a,b,cc),(a,cc,d)))
local=np.asarray(local);facearray=np.asarray(faces);chains=[]
for phase in np.arange(0,7,.5):
    vv=[];ff=[]
    for i in range(params['count']):
        p,r=pose(i*spacing+float(phase)/c,i);off=len(vv);vv.extend(local@np.asarray(r).T+np.asarray(p));ff.extend(facearray+off)
    vv=np.asarray(vv);ff=np.asarray(ff);chains.append((float(phase),BVHTree.FromPolygons(vv,ff,all_triangles=True),vv[ff]))
failures=[];tested=0
for frame in range(45):
    scene.frame_set(frame*2);bpy.context.view_layer.update();body,bt,names=tree(objects)
    for phase,chain,ct in chains:
        pairs=chain.overlap(body);counts={};tested+=1
        if pairs:
            ai,bi=np.asarray(pairs).T;mask=crosses(ct[ai],bt[bi])|crosses(bt[bi],ct[ai])
            for name in np.asarray(names)[bi[mask]]:counts[str(name)]=counts.get(str(name),0)+1
        if counts:failures.append({'source_frame':frame,'phase_native_pixels':phase,'exact_body_crossing_pairs':counts})
result={'status':'Private contact diagnostic; no approval','source_model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'path':params,'body_poses':45,'chain_phases':14,'pose_phase_pairs_tested':tested,'failures':failures,'limitations':['Fixed source strand centers; source fit and inferred attachment still require review.','Exact edge crossings do not detect complete containment.','Finite phase samples do not prove continuous swept clearance.']};OUT.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'tested':tested,'failed':len(failures),'examples':failures[:5]},indent=2))
