"""Package the checked inferred hardware while preserving all frozen winch parts."""
import ast,hashlib,json,math,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-guided-entry-candidate-v1';OUT=WORK/'restart2/winch-supported-hardware-v1'
if OUT.exists():raise FileExistsError(OUT)
def budget():
    used=sum(p.stat().st_size for b in (BASE,OUT) for p in b.rglob('*') if p.is_file())
    assert used<20*1024**2 and shutil.disk_usage(ROOT).free>10*1024**3+20*1024**2-used
budget();audit_path=WORK/'restart2/winch-hardware-envelopes-v6.json';audit=json.loads(audit_path.read_text())
assert all(not audit[k] for k in ('static_chain_crossings','static_room_crossings','body_contacts','collar_chain_crossings','chain_body_crossings','chain_room_crossings'))
assert min(r['minimum_adjacent_wire_clearance_bound'] for r in audit['adjacent_link_clearances'])>0
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,bmesh,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1));screen_side=Vector((1,0,0));back=Vector((0,-c,s));spacing=3.5/c
recipes=[]
for name in ('restart13_winch_return_study.py','restart14_winch_guided_entry.py','restart15_winch_hardware_envelopes.py'):
    recipe=Path(__file__).with_name(name);recipes.append({'path':str(recipe),'sha256':hashlib.sha256(recipe.read_bytes()).hexdigest()});exec(compile(ast.Module(body=[n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(recipe),'exec'))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0));pose,params,path=guided_route(count=76)
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;scope=set(json.loads((WORK/'restart2/winch-components-source-v2/component-freeze.json').read_text())['scope']);body=[o for o in scene.objects if o.name in scope];guards=[]
for frame in range(45):
    scene.frame_set(frame*2);bpy.context.view_layer.update();guards.append({o.name:_geometry(o,protect_appearance=True) for o in body})
scene.frame_set(88);bpy.context.view_layer.update();links=sorted((o for o in scene.objects if o.name.startswith('Guided chain link')),key=lambda o:o.name);outside={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in links};material=links[0].data.materials[0]
while len(links)<76:
    o=links[0].copy();o.name=f'Guided chain link {len(links):03d}';scene.collection.objects.link(o);links.append(o)
for i,o in enumerate(links):p,r=pose(i*spacing+3.5/c,i);o.location=p;o.rotation_euler=r.to_euler()
bpy.context.view_layer.update();context=[o for o in scene.objects if o.type=='MESH' and o.get('native_patch')!='patch-004' and not o.hide_render];room,roomtri,roomnames=tree(context)
# Reuse the exact measured construction statements, excluding audit loops and IO.
recipe=Path(__file__).with_name('restart15_winch_hardware_envelopes.py');source=recipe.read_text();start=source[:source.index('upper=curve_guide(')].count('\n')+1;end=source[:source.index('def collar(')].count('\n')+1;nodes=[n for n in ast.parse(source).body if start<=n.lineno<end];exec(compile(ast.Module(body=nodes,type_ignores=[]),str(recipe),'exec'))
parts.update(collar(scene.objects['Travelling round part solid drum'].matrix_world.translation.z));created=[];travel_parent=scene.objects['Winch travelling part physical descent']
for name,(vv,ff) in parts.items():
    mesh=bpy.data.meshes.new('Inferred '+name);mesh.from_pydata(vv.tolist(),[],ff.tolist());mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free();o=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(o);o.data.materials.append(material);o['source_node']='scenery-york-castle-winch';o['asset_group']='york-castle-winch';o['native_patch']='patch-004';o['inferred_hardware']=True
    if name.startswith('traveller'):
        o.parent=travel_parent;o.matrix_parent_inverse=travel_parent.matrix_world.inverted()
    created.append(o)
bpy.context.view_layer.update();assert outside=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in outside};guarded=0
for frame in range(45):
    scene.frame_set(frame*2);bpy.context.view_layer.update();assert guards[frame]=={o.name:_geometry(o,protect_appearance=True) for o in body};guarded+=1
scene.frame_set(88);bpy.context.view_layer.update();budget();OUT.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True);assert (OUT/'model.blend').stat().st_size<8*1024**2;budget()
record={'status':'Private inferred hardware candidate; not geometry/texture/mechanism approved','source_model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'checked_recipe_files':recipes,'cpu_audit':{'path':str(audit_path),'sha256':hashlib.sha256(audit_path.read_bytes()).hexdigest()},'frozen_component_count':len(body),'frozen_pose_guards':guarded,'outside_exact_at_final':len(outside),'hardware_objects':[o.name for o in created],'chain_path':params,'chain_phase_native_pixels':3.5,'ceiling_anchors':anchors,'frame_anchors':frame_anchors,'inferences':['Upper return and fixed guide shoes are hidden-depth inferences with measured ceiling/timber anchors.','Sliding collar follows the measured round-part path independently of material link identity; brake/drive is not established by the source.'],'remaining':['Review saved-model eight views, source and physical contacts.','Project source pixels onto the small visible collar/arm region; all other new hardware was hidden in the sampled native views.','Complete chain phase animation, source-hole exceptions, appearance and final assembly/export remain pending.','Finite56phase contact checks do not prove continuous clearance or containment.']};(OUT/'proposal.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps({'model_sha256':record['model_sha256'],'model_bytes':(OUT/'model.blend').stat().st_size,'hardware':len(created),'guarded_poses':guarded}))
