"""Identify the visible owner at each unresolved guided-chain source hole."""
import ast,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'winch-guided-entry-candidate-v1';OUT=WORK/'winch-guided-entry-source-fit-fine-v1/hole-owners.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));up=Vector((0,0,1));screen_side=Vector((1,0,0));spacing=3.5/c
for name in ('restart13_winch_return_study.py','restart14_winch_guided_entry.py'):
    p=Path(__file__).with_name(name);exec(compile(ast.Module(body=[n for n in ast.parse(p.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(p),'exec'))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0));pose,params,path=guided_route()
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;links=sorted((o for o in scene.objects if o.name.startswith('Guided chain link')),key=lambda o:o.name);fit=json.loads((OUT.parent/'fit.json').read_text());ha=json.loads((WORK/'winch-motion-physical-v2/hole-owner-audit-v2.json').read_text());rows=[]
for record in fit['frames']:
    frame=record['frame'];phase=record['best']['phase_game'];scene.frame_set(frame*2)
    for i,o in enumerate(links):p,r=pose(i*spacing+phase/c,i);o.location=p;o.rotation_euler=r.to_euler()
    bpy.context.view_layer.update();vertices=[];faces=[];owners=[]
    for o in scene.objects:
        if o.type!='MESH' or o.hide_render:continue
        offset=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices)
        for face in o.data.polygons:faces.append(tuple(offset+j for j in face.vertices));owners.append((o.name,o.get('native_patch')=='patch-004'))
    mesh=BVHTree.FromPolygons(vertices,faces);holes=[]
    for hole in ha['frames'][frame]['holes']:
        x,y=hole['native_pixel'];origin=Vector((x+.5,-(y+.5)/s,0))+back*10000;hit=mesh.ray_cast(origin,-back);owner,filled=owners[hit[2]] if hit[2] is not None else (None,False);holes.append({'native_pixel':[x,y],'area':hole['area'],'owner':owner,'filled_by_mechanism':filled})
    rows.append({'frame':frame,'phase_native_pixels':phase,'holes':holes})
OUT.write_text(json.dumps({'status':'Unresolved source hole diagnosis, no changes','frames':rows},indent=2)+'\n');print(json.dumps(rows,indent=2))
