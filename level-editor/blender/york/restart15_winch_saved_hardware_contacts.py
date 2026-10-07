"""Recheck serialized hardware geometry rather than relying on CPU proposal arrays."""
import ast,hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'winch-supported-hardware-v1';OUT=BASE/'saved-contact-check.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1));screen_side=Vector((1,0,0));spacing=3.5/c
for name in ('restart13_winch_return_study.py','restart14_winch_guided_entry.py','restart15_winch_hardware_envelopes.py'):
    p=Path(__file__).with_name(name);exec(compile(ast.Module(body=[n for n in ast.parse(p.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(p),'exec'))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0));pose,params,path=guided_route(count=76)
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scope=set(json.loads((WORK/'winch-components-source-v2/component-freeze.json').read_text())['scope']);body=[o for o in scene.objects if o.name in scope];hardware=[o for o in scene.objects if o.get('inferred_hardware')];moving=[o for o in hardware if o.name.startswith('Inferred traveller')];static=[o for o in hardware if o not in moving];links=sorted((o for o in scene.objects if o.name.startswith('Guided chain link')),key=lambda o:o.name);context=[o for o in scene.objects if o.type=='MESH' and o.get('native_patch')!='patch-004' and not o.hide_render];scene.frame_set(88);bpy.context.view_layer.update();room,rt,rn=tree(context);st,stt,stn=tree(static);room_hits=intersections(st,stt,stn,room,rt,rn);body_hits=[];hardware_hits=[];chain_hits=[]
chains=[]
for phase in np.arange(0,7,.125):
    for i,o in enumerate(links):p,r=pose(i*spacing+float(phase)/c,i);o.location=p;o.rotation_euler=r.to_euler()
    bpy.context.view_layer.update();chains.append((float(phase),*tree(links)))
for frame in range(45):
    scene.frame_set(frame*2);bpy.context.view_layer.update();bt,btt,btn=tree(body);mt,mtt,mtn=tree(moving);ht,htt,htn=tree(hardware);hits=intersections(ht,htt,htn,bt,btt,btn)
    if hits:body_hits.append({'frame':frame,'pairs':hits})
    hits=intersections(mt,mtt,mtn,st,stt,stn)
    if hits:hardware_hits.append({'frame':frame,'pairs':hits})
    for phase,ct,ctt,ctn in chains:
        hits=intersections(ht,htt,htn,ct,ctt,ctn)
        if hits:chain_hits.append({'frame':frame,'phase':phase,'pairs':hits})
result={'status':'Saved geometry contact check; finite samples, not continuous proof','model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'hardware_objects':[o.name for o in hardware],'body_poses':45,'chain_phases':56,'hardware_room_hits':room_hits,'hardware_body_hits':body_hits,'moving_static_hardware_hits':hardware_hits,'hardware_chain_hits':chain_hits,'limitations':['Triangle edge crossings do not test complete containment.','Fixed support-to-body or support-to-ceiling interfaces have a nominal1e-4world clearance tolerance.']};OUT.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'hardware_room':room_hits,'body_failed_poses':len(body_hits),'moving_static_failed_poses':len(hardware_hits),'hardware_chain_failed_pairs':len(chain_hits),'body_examples':body_hits[:2],'chain_examples':chain_hits[:2]},indent=2))
