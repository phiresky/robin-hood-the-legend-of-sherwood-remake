"""Reopen the motion model and check actual saved poses and contacts."""
import ast,hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'winch-supported-motion-v1';OUT=BASE/'saved-motion-check.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1));screen_side=Vector((1,0,0));spacing=3.5/c
for name in ('restart13_winch_return_study.py','restart14_winch_guided_entry.py','restart15_winch_hardware_envelopes.py'):
    p=Path(__file__).with_name(name);exec(compile(ast.Module(body=[n for n in ast.parse(p.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(p),'exec'))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0));pose,params,path=guided_route(count=76)
plan=json.loads((WORK/'winch-supported-motion-plan-v1.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(WORK/'winch-supported-hardware-v1/model.blend'));scene=bpy.context.scene;guards=[]
for row in plan['rows']:
    scene.frame_set(row['tick']);bpy.context.view_layer.update();guards.append({o.name:_geometry(o,protect_appearance=True) for o in scene.objects if not o.name.startswith('Guided chain link')})
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scope=set(json.loads((WORK/'winch-components-source-v2/component-freeze.json').read_text())['scope']);body=[o for o in scene.objects if o.name in scope];hardware=[o for o in scene.objects if o.get('inferred_hardware')];links=sorted((o for o in scene.objects if o.name.startswith('Guided chain link')),key=lambda o:o.name);context=[o for o in scene.objects if o.type=='MESH' and o.get('native_patch')!='patch-004' and not o.hide_render];room,rt,rn=tree(context);rows=[]
for index,row in enumerate(plan['rows']):
    scene.frame_set(row['tick']);bpy.context.view_layer.update();assert guards[index]=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in links};max_position_error=0.;max_rotation_error=0.
    for i,obj in enumerate(links):
        p,r=pose(i*spacing+row['phase_native_pixels']/c,i);max_position_error=max(max_position_error,(obj.location-p).length);expected=r.to_euler().to_matrix();actual=obj.rotation_euler.to_matrix();max_rotation_error=max(max_rotation_error,max(abs(actual[a][b]-expected[a][b]) for a in range(3) for b in range(3)))
    assert max_position_error<1e-5 and max_rotation_error<1e-5
    ct,ctt,ctn=tree(links);ht,htt,htn=tree(hardware);bt,btt,btn=tree(body)
    hits={'chain_hardware':intersections(ct,ctt,ctn,ht,htt,htn),'chain_body':intersections(ct,ctt,ctn,bt,btt,btn),'chain_room':intersections(ct,ctt,ctn,room,rt,rn)}
    assert not any(hits.values()),(row,hits)
    rows.append({'frame':index,'phase_native_pixels':row['phase_native_pixels'],'max_position_error':max_position_error,'max_rotation_matrix_error':max_rotation_error,'crossings':hits})
    held={o.name:[list(r) for r in o.matrix_world] for o in links};scene.frame_set(row['tick']+1);bpy.context.view_layer.update();assert held=={o.name:[list(r) for r in o.matrix_world] for o in links}
result={'status':'PASS saved45poses and per-tick holds; finite intersections only, no motion approval','model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'all63_protected_exact_all45':True,'rows':rows,'limitations':['Finite actual keyframes, not continuous collision proof or containment test.','Inferred periodic phase representative and independent sliding collar; no observed drive ratio.']};OUT.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'status':'PASS','saved_poses':len(rows),'protected_objects':len(guards[0])}))
